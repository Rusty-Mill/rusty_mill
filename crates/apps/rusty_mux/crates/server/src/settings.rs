//! The server's part of the config file: its `server` section, which sets
//! what a new block runs and where (C38).

use std::io::{self, Read};
use std::path::Path;

use rusty_mux_script::Defaults;

use crate::Config;

/// The largest config file the server reads.
const LIMIT: u64 = 1 << 20;

/// `base` with the `server` section of its config file over it: `base`
/// itself when there is no file or it has no section (so a file deleted
/// since brings back the server's own defaults). Only that section is
/// checked; the rest is ctl's. The error names the file and lists every
/// problem, one per line; the caller keeps what it had.
pub(crate) fn read(base: &Config) -> Result<Config, String> {
    let Some(path) = &base.config_file else {
        return Ok(base.clone());
    };
    let text = match bounded(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(base.clone()),
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
/// file: opening a FIFO would block the server's one thread until a writer
/// came. Read in one blocking call, as the file is small.
fn bounded(path: &Path) -> io::Result<String> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > LIMIT {
        return Err(io::Error::other(format!("larger than {LIMIT} bytes")));
    }
    Ok(text)
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
        assert_eq!(read(&base(None)), Ok(base(None)));
        let missing = base(Some(
            std::env::temp_dir().join("rusty-mux-settings-none.json"),
        ));
        assert_eq!(read(&missing), Ok(missing.clone()));
        let empty = base(Some(temp("empty", "{}")));
        assert_eq!(read(&empty), Ok(empty.clone()));
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
        let read = read(&base(Some(temp("set", &text)))).unwrap();
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

    fn read_ok(path: PathBuf) -> Config {
        read(&base(Some(path))).unwrap()
    }

    #[test]
    fn a_bad_or_huge_file_is_refused_with_every_problem() {
        let path = temp("bad", r#"{"server": {"command": "", "cwd": "rel"}}"#);
        let err = read(&base(Some(path.clone()))).unwrap_err();
        assert_eq!(err.lines().count(), 2, "{err}");
        assert!(err.starts_with(&path.display().to_string()), "{err}");

        let huge = temp("huge", &" ".repeat(LIMIT as usize + 1));
        assert!(read(&base(Some(huge))).unwrap_err().contains("larger than"));

        // A directory that is not there, and a path that is no file.
        let gone = std::env::temp_dir().join(format!("rusty-mux-gone-{}", std::process::id()));
        let json = format!("\"{}\"", gone.display().to_string().replace('\\', "\\\\"));
        let path = temp("gone", &format!(r#"{{"server": {{"cwd": {json}}}}}"#));
        assert!(
            read(&base(Some(path)))
                .unwrap_err()
                .contains("not a directory")
        );
        let dir = base(Some(std::env::temp_dir()));
        assert!(read(&dir).unwrap_err().contains("not a regular file"));

        // Other sections are ctl's: one this build cannot check is no
        // reason to refuse.
        let path = temp("ctl", r#"{"on": [{"event": "from-the-future"}]}"#);
        assert!(read(&base(Some(path))).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_fifo_is_refused_without_waiting_for_a_writer() {
        let path = std::env::temp_dir().join(format!("rusty-mux-fifo-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let made = std::process::Command::new("mkfifo").arg(&path).status();
        assert!(made.expect("mkfifo").success());
        let err = read(&base(Some(path.clone()))).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(err.contains("not a regular file"), "{err}");
    }
}
