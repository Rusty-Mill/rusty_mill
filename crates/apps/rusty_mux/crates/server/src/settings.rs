//! The server's part of the config file: its `server` section, which sets
//! what a new block runs and where (C38).

use std::io::{self, Read};
use std::path::Path;

use rusty_mux_script::Defaults;

use crate::Config;

/// The largest config file the server reads.
const LIMIT: u64 = 1 << 20;

/// `base` with the `server` section of its config file over it: `base`
/// itself when there is no file or it has no section. The error names the
/// file and lists every problem, one per line; the caller keeps what it
/// had.
pub(crate) fn read(base: &Config) -> Result<Config, String> {
    let Some(path) = &base.config_file else {
        return Ok(base.clone());
    };
    let text = match bounded(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(base.clone()),
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    let parsed = rusty_mux_script::parse(&text).map_err(|problems| {
        let lines: Vec<String> = problems
            .iter()
            .map(|p| format!("{}: {p}", path.display()))
            .collect();
        lines.join("\n")
    })?;
    Ok(overlay(base, &parsed.server))
}

/// The text of `path`, refused past [`LIMIT`]. Read in one blocking call:
/// the file is local and small.
fn bounded(path: &Path) -> io::Result<String> {
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
    }
}
