//! The server's part of the config file: its `server` section, which sets
//! what a new block runs and where (C38).

use std::io::{self, Read};
use std::path::Path;

use rusty_mux_script::Defaults;

use crate::Config;

/// The largest config file the server reads.
const LIMIT: u64 = 1 << 20;

/// When the file is read, which decides what a missing one means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum When {
    /// At start: a missing file sets nothing.
    Start,
    /// On `reload-config`: a missing file is refused, so one deleted or
    /// mid-replace never undoes the last good settings.
    Reload,
}

/// `base` with the `server` section of its config file over it: `base`
/// itself when there is no file or it has no section. Only that section is
/// checked; the rest is ctl's. The error names the file and lists every
/// problem, one per line; the caller keeps what it had.
pub(crate) fn read(base: &Config, when: When) -> Result<Config, String> {
    let Some(path) = &base.config_file else {
        return Ok(base.clone());
    };
    let text = match bounded(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound && when == When::Start => {
            return Ok(base.clone());
        }
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    let defaults = rusty_mux_script::parse_server(&text).map_err(|problems| {
        let lines: Vec<String> = problems
            .iter()
            .map(|p| format!("{}: {p}", path.display()))
            .collect();
        lines.join("\n")
    })?;
    // As a request's cwd: a directory that is not there would only fail
    // each block later, after the reload was announced as taken.
    if let Some(cwd) = &defaults.cwd
        && !Path::new(cwd).is_dir()
    {
        return Err(format!(
            "{}: server.cwd: `{cwd}` is not a directory",
            path.display()
        ));
    }
    Ok(overlay(base, &defaults))
}

/// The text of `path`, refused past [`LIMIT`] or when it is not a regular
/// file. Read in one blocking call on the server's thread, as a local
/// regular file is small and quick; a file on a stalled network mount
/// would still stall it.
fn bounded(path: &Path) -> io::Result<String> {
    let mut text = String::new();
    open_regular(path)?
        .take(LIMIT + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > LIMIT {
        return Err(io::Error::other(format!("larger than {LIMIT} bytes")));
    }
    Ok(text)
}

/// `path` opened for reading if it is a regular file. On Linux it is
/// opened non-blocking, which a regular file ignores, so a FIFO put in its
/// place cannot hold the open until a writer comes; the check is on the
/// opened file, so a swap after it changes nothing.
fn open_regular(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rusty_libc::fd::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    Ok(file)
}

/// `base` with what `defaults` sets: a command replaces the program and
/// its arguments together, a directory the working directory.
fn overlay(base: &Config, defaults: &Defaults) -> Config {
    let mut config = base.clone();
    if let Some(command) = &defaults.command {
        config.program = command.into();
        config.args = defaults.args.iter().map(Into::into).collect();
    }
    if let Some(cwd) = &defaults.cwd {
        config.cwd = cwd.into();
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConsoleHost;
    use std::path::PathBuf;

    fn base(file: Option<PathBuf>) -> Config {
        Config {
            program: "/bin/sh".into(),
            args: vec!["-l".into()],
            cwd: "/".into(),
            console_host: ConsoleHost::default(),
            config_file: file,
        }
    }

    fn temp(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rusty-mux-settings-{name}-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn no_file_or_no_section_keeps_the_base() {
        assert_eq!(read(&base(None), When::Reload), Ok(base(None)));
        let missing = base(Some(
            std::env::temp_dir().join("rusty-mux-settings-none.json"),
        ));
        assert_eq!(read(&missing, When::Start), Ok(missing.clone()));
        let empty = base(Some(temp("empty", "{}")));
        assert_eq!(read(&empty, When::Reload), Ok(empty.clone()));
    }

    #[test]
    fn a_reload_refuses_a_missing_file() {
        let missing = base(Some(
            std::env::temp_dir().join("rusty-mux-settings-none.json"),
        ));
        let err = read(&missing, When::Reload).unwrap_err();
        assert!(err.contains("rusty-mux-settings-none.json"), "{err}");
    }

    /// An absolute directory on any platform, and its JSON string.
    fn absolute() -> (String, String) {
        let dir = std::env::temp_dir().display().to_string();
        let json = format!("\"{}\"", dir.replace('\\', "\\\\"));
        (dir, json)
    }

    #[test]
    fn the_section_replaces_command_and_directory() {
        let (dir, json) = absolute();
        let text =
            format!(r#"{{"server": {{"command": "/bin/bash", "args": ["-i"], "cwd": {json}}}}}"#);
        let read = read_at(&base(Some(temp("set", &text)))).unwrap();
        assert_eq!(read.program, "/bin/bash");
        assert_eq!(read.args, vec![std::ffi::OsString::from("-i")]);
        assert_eq!(read.cwd, dir.as_str());

        // A command alone drops the base's arguments; a directory alone
        // keeps its program.
        let path = temp("cmd", r#"{"server": {"command": "/bin/zsh"}}"#);
        assert!(read_ok(path).args.is_empty());
        let path = temp("cwd", &format!(r#"{{"server": {{"cwd": {json}}}}}"#));
        let cwd = read_ok(path);
        assert_eq!(
            (cwd.program.as_os_str(), cwd.cwd.as_os_str()),
            ("/bin/sh".as_ref(), dir.as_ref())
        );
    }

    fn read_at(config: &Config) -> Result<Config, String> {
        read(config, When::Reload)
    }

    fn read_ok(path: PathBuf) -> Config {
        read_at(&base(Some(path))).unwrap()
    }

    #[test]
    fn a_bad_or_huge_file_is_refused_with_every_problem() {
        let path = temp("bad", r#"{"server": {"command": "", "cwd": "rel"}}"#);
        let err = read_at(&base(Some(path.clone()))).unwrap_err();
        assert_eq!(err.lines().count(), 2, "{err}");
        assert!(err.starts_with(&path.display().to_string()), "{err}");

        let huge = temp("huge", &" ".repeat(LIMIT as usize + 1));
        assert!(
            read_at(&base(Some(huge)))
                .unwrap_err()
                .contains("larger than")
        );

        // A directory that is not there, and a path that is no file.
        let gone = std::env::temp_dir().join(format!("rusty-mux-gone-{}", std::process::id()));
        let json = format!("\"{}\"", gone.display().to_string().replace('\\', "\\\\"));
        let path = temp("gone", &format!(r#"{{"server": {{"cwd": {json}}}}}"#));
        assert!(
            read_at(&base(Some(path)))
                .unwrap_err()
                .contains("not a directory")
        );
        let dir = base(Some(std::env::temp_dir()));
        // Linux opens a directory and finds it is no file; Windows will
        // not open it.
        assert!(read_at(&dir).is_err());

        // Other sections are ctl's: one this build cannot check is no
        // reason to refuse.
        let path = temp("ctl", r#"{"on": [{"event": "from-the-future"}]}"#);
        assert!(read_at(&base(Some(path))).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_fifo_is_refused_without_waiting_for_a_writer() {
        let path = std::env::temp_dir().join(format!("rusty-mux-fifo-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let made = std::process::Command::new("mkfifo").arg(&path).status();
        assert!(made.expect("mkfifo").success());
        let err = read_at(&base(Some(path.clone()))).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(err.contains("not a regular file"), "{err}");
    }
}
