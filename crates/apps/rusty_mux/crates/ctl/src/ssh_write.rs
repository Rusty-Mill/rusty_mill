//! `ssh setup --write` and `ssh remove` (C48, pack D193): our Host blocks
//! live in our own file, `~/.ssh/rusty_mux/config`, which one marked
//! `Include` line at the top of `~/.ssh/config` reads. The text edits are
//! pure; [`apply`] does the file work: a backup of the user's config before
//! our first edit of it, a check of our file with `ssh -G -F`, and
//! temp-file-and-rename writes. A symlinked config is refused.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The comment that marks our Include line, the line just above it.
const MARK: &str = "# Added by rusty-mux-ctl ssh setup; `rusty-mux-ctl ssh remove` takes it out.";
/// The line that reads our file. `~` works in OpenSSH on every platform.
const INCLUDE: &str = "Include ~/.ssh/rusty_mux/config";

/// The two files `setup --write` and `remove` touch.
pub(crate) struct Files {
    /// The user's `~/.ssh/config`.
    pub(crate) config: PathBuf,
    /// Our `~/.ssh/rusty_mux/config`.
    pub(crate) ours: PathBuf,
}
impl Files {
    pub(crate) fn under(ssh_dir: &Path) -> Self {
        Files {
            config: ssh_dir.join("config"),
            ours: ssh_dir.join("rusty_mux").join("config"),
        }
    }
    fn backup(&self) -> PathBuf {
        self.config.with_file_name("config.rusty-mux.bak")
    }
}

/// What a write did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Done {
    /// The files already said so.
    Nothing,
    /// The files changed; with `Some`, the user's config was first copied
    /// there.
    Changed { backup: Option<PathBuf> },
}

/// The markers around `dest`'s block (see [`crate::ssh::block`]).
fn markers(dest: &str) -> (String, String) {
    (
        format!("# BEGIN rusty-mux-ctl ssh setup {dest}"),
        format!("# END rusty-mux-ctl ssh setup {dest}"),
    )
}

/// `ours` without `dest`'s block. A BEGIN without its END is left alone.
pub(crate) fn without_block(ours: &str, dest: &str) -> String {
    let (begin, end) = markers(dest);
    let lines: Vec<&str> = ours.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == begin) else {
        return ours.to_owned();
    };
    let Some(len) = lines[start..].iter().position(|l| l.trim() == end) else {
        return ours.to_owned();
    };
    let kept: Vec<&str> = lines[..start]
        .iter()
        .chain(&lines[start + len + 1..])
        .copied()
        .collect();
    joined(&kept)
}

/// `ours` with `block` as `dest`'s block, in place of an old one or last.
pub(crate) fn with_block(ours: &str, dest: &str, block: &str) -> String {
    let rest = without_block(ours, dest);
    let mut lines: Vec<&str> = rest.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.extend(block.lines());
    joined(&lines)
}

/// Why `dest` cannot name a Host block we write, if it cannot: only a
/// plain alias, so the block matches that one name and no pattern (`*`,
/// `?`, `!`) or `user@host` slips in ahead of the user's own config.
pub(crate) fn alias(dest: &str) -> Result<(), String> {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
    if dest.is_empty() || dest.starts_with('-') || !dest.chars().all(plain) {
        return Err(format!(
            "`{dest}`: --write takes a plain host alias (letters, digits, `.`, `-`, `_`); \
             set the user with --user"
        ));
    }
    Ok(())
}

/// How many bytes at the start of `config` are ours: the mark, the
/// Include line, and the blank line [`with_include`] puts after them.
/// Any line ending is accepted, so an editor's CRLF still counts.
fn ours_at_top(config: &str) -> Option<usize> {
    let mut lines = config.split_inclusive('\n');
    let mark = lines.next()?;
    let include = lines.next()?;
    if mark.trim_end() != MARK || include.trim_end() != INCLUDE {
        return None;
    }
    let blank = lines
        .next()
        .filter(|l| l.trim().is_empty())
        .map_or(0, str::len);
    Some(mark.len() + include.len() + blank)
}

/// `config` with our marked Include line at the top, or `None` when it
/// already starts with it. At the top, ahead of any `Host` or `Match`, it
/// applies to every host.
pub(crate) fn with_include(config: &str) -> Option<String> {
    if ours_at_top(config).is_some() {
        return None;
    }
    Some(format!("{MARK}\n{INCLUDE}\n\n{config}"))
}

/// `config` without the lines [`with_include`] added at its top; every
/// other byte is kept, including an Include line the user wrote.
pub(crate) fn without_include(config: &str) -> String {
    match ours_at_top(config) {
        Some(len) => config[len..].to_owned(),
        None => config.to_owned(),
    }
}

/// Lines joined with a final newline; nothing for no lines.
fn joined(lines: &[&str]) -> String {
    match lines {
        [] => String::new(),
        _ => format!("{}\n", lines.join("\n")),
    }
}

/// Puts `block` for `dest` in our file and the Include line in the user's
/// config. Our file is checked with `ssh -G -F` before it replaces the old.
pub(crate) fn setup(files: &Files, dest: &str, block: &str, ssh: &OsStr) -> Result<Done, String> {
    let ours = read(&files.ours)?;
    let config = read_config(&files.config)?;
    let new_ours = with_block(&ours, dest, block);
    let new_config = with_include(&config);
    if new_ours == ours && new_config.is_none() {
        return Ok(Done::Nothing);
    }
    let temp = write_temp(&files.ours, &new_ours)?;
    if let Err(reason) = check(ssh, &temp, dest) {
        let _gone = std::fs::remove_file(&temp);
        return Err(reason);
    }
    rename(&temp, &files.ours)?;
    let backup = match new_config {
        Some(text) => replace_config(files, &config, &text)?,
        None => None,
    };
    Ok(Done::Changed { backup })
}

/// Takes `dest`'s block out of our file. Once it holds no block, the file
/// and the Include line go too.
pub(crate) fn remove(files: &Files, dest: &str) -> Result<Done, String> {
    let ours = read(&files.ours)?;
    let new_ours = without_block(&ours, dest);
    let config = read_config(&files.config)?;
    let empty = new_ours.trim().is_empty();
    let new_config = without_include(&config);
    if new_ours == ours && (!empty || new_config == config) {
        return Ok(Done::Nothing);
    }
    match empty {
        true => match std::fs::remove_file(&files.ours) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(failed(&files.ours, e)),
            _ => {}
        },
        false => {
            let temp = write_temp(&files.ours, &new_ours)?;
            rename(&temp, &files.ours)?;
        }
    }
    let backup = match empty && new_config != config {
        true => replace_config(files, &config, &new_config)?,
        false => None,
    };
    Ok(Done::Changed { backup })
}

/// Writes the user's config as `text`, after a backup of `old` the first
/// time only, so the backup is the config from before rusty-mux-ctl.
fn replace_config(files: &Files, old: &str, text: &str) -> Result<Option<PathBuf>, String> {
    let backup = files.backup();
    let made = match files.config.exists() {
        // `create_new`: an existing backup, or a link where it would go,
        // is left alone.
        true => match create_private(&backup, old) {
            Ok(()) => Some(backup),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
            Err(e) => return Err(failed(&backup, e)),
        },
        false => None,
    };
    let temp = write_temp(&files.config, text)?;
    rename(&temp, &files.config)?;
    Ok(made)
}

/// `ssh -G -F file DEST`: whether ssh reads `file` without error.
fn check(ssh: &OsStr, file: &Path, dest: &str) -> Result<(), String> {
    let output = Command::new(ssh)
        .arg("-G")
        .arg("-F")
        .arg(file)
        .args(["--", dest])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", ssh.to_string_lossy()))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "ssh does not accept the new block, so nothing was written: {}",
        said.trim()
    ))
}

/// The user's config, refusing a symlink: writing through one would
/// replace the link with a file.
fn read_config(path: &Path) -> Result<String, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "{} is a symlink; add `{INCLUDE}` to the file it points to yourself",
            path.display()
        )),
        _ => read(path),
    }
}

/// A file's text; a missing file is empty.
fn read(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(failed(path, e)),
    }
}

/// `text` in a private temp file beside `path`, whose folder is made
/// private if missing.
fn write_temp(path: &Path, text: &str) -> Result<PathBuf, String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("{}: no directory", path.display()))?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir).map_err(|e| failed(dir, e))?;
    let name = path.file_name().unwrap_or(OsStr::new("config"));
    let temp = dir.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    write_private(&temp, text).map_err(|e| {
        let _gone = std::fs::remove_file(&temp);
        failed(&temp, e)
    })?;
    Ok(temp)
}

/// Moves `temp` over `path`; on failure `temp` is removed too, as far as
/// it can be.
fn rename(temp: &Path, path: &Path) -> Result<(), String> {
    std::fs::rename(temp, path).map_err(|e| {
        let left = match std::fs::remove_file(temp) {
            Ok(()) => String::new(),
            Err(gone) => format!("; {} is left: {gone}", temp.display()),
        };
        format!("{}{left}", failed(path, e))
    })
}

/// `text` in a new private file at `path`; `AlreadyExists` if anything,
/// even a dangling link, is there.
fn create_private(path: &Path, text: &str) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    write_with(options, path, text)
}

fn write_private(path: &Path, text: &str) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    write_with(options, path, text)
}

fn write_with(options: std::fs::OpenOptions, path: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    let options = {
        let mut options = options;
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options
    };
    let mut file = options.open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

fn failed(path: &Path, e: io::Error) -> String {
    format!("{}: {e}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK_A: &str =
        "# BEGIN rusty-mux-ctl ssh setup a\nHost a\n    User me\n# END rusty-mux-ctl ssh setup a";
    const BLOCK_B: &str =
        "# BEGIN rusty-mux-ctl ssh setup b\nHost b\n# END rusty-mux-ctl ssh setup b";

    #[test]
    fn a_block_is_added_once_and_replaced_in_place() {
        let one = with_block("", "a", BLOCK_A);
        assert_eq!(one, format!("{BLOCK_A}\n"));
        assert_eq!(with_block(&one, "a", BLOCK_A), one, "twice changes nothing");
        let two = with_block(&one, "b", BLOCK_B);
        let changed = BLOCK_A.replace("User me", "User you");
        let three = with_block(&two, "a", &changed);
        assert!(three.contains("User you") && !three.contains("User me"));
        assert!(three.contains("Host b"));
    }

    #[test]
    fn removing_a_block_keeps_the_others() {
        let both = with_block(&with_block("", "a", BLOCK_A), "b", BLOCK_B);
        assert_eq!(without_block(&both, "a"), format!("{BLOCK_B}\n"));
        assert_eq!(without_block(&both, "c"), both);
        let open = "# BEGIN rusty-mux-ctl ssh setup a\nHost a\n";
        assert_eq!(without_block(open, "a"), open, "no END: left alone");
    }

    #[test]
    fn the_include_goes_on_top_once_and_comes_out_cleanly() {
        let config = "Host *\n    ServerAliveInterval 30\n";
        let with = with_include(config).expect("added");
        assert!(with.starts_with(&format!("{MARK}\n{INCLUDE}\n")));
        assert!(with_include(&with).is_none(), "once");
        assert_eq!(without_include(&with), config);
        assert_eq!(
            with_include("").as_deref(),
            Some(&*format!("{MARK}\n{INCLUDE}\n\n"))
        );
    }

    #[test]
    fn remove_keeps_every_byte_but_ours() {
        // CRLF, no final newline, and the user's own identical Include
        // inside a Match block: all theirs.
        let config = "\r\nMatch host x\r\n    Include ~/.ssh/rusty_mux/config\r\nHost y";
        let with = with_include(config).expect("theirs is not ours at the top");
        assert_eq!(without_include(&with), config);
        assert_eq!(
            without_include(config),
            config,
            "nothing of ours: untouched"
        );
        let edited = format!("{MARK}\r\n{INCLUDE}\r\n\r\nHost z\r\n");
        assert_eq!(without_include(&edited), "Host z\r\n", "an editor's CRLF");
    }

    #[test]
    fn only_a_plain_alias_is_written() {
        for good in ["box", "my-box.lan", "box_2"] {
            assert!(alias(good).is_ok(), "{good}");
        }
        for bad in ["*", "b?x", "!box", "me@box", "-box", "a b", "a,b", ""] {
            assert!(alias(bad).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    mod files {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn scratch(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "rusty-mux-ssh-write-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _fresh = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("made");
            dir
        }

        /// An `ssh` that accepts every file, or refuses every one.
        fn fake_ssh(dir: &Path, ok: bool) -> PathBuf {
            let path = dir.join(if ok { "ssh-ok" } else { "ssh-no" });
            let body = match ok {
                true => "#!/bin/sh\nexit 0\n",
                false => "#!/bin/sh\necho 'bad option' >&2\nexit 255\n",
            };
            std::fs::write(&path, body).expect("written");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
            path
        }

        #[test]
        fn setup_backs_up_once_and_remove_undoes_it() {
            let dir = scratch("cycle");
            let files = Files::under(&dir.join(".ssh"));
            std::fs::create_dir_all(dir.join(".ssh")).expect("made");
            std::fs::write(&files.config, "Host old\n").expect("written");
            let ssh = fake_ssh(&dir, true);
            let done = setup(&files, "a", BLOCK_A, ssh.as_os_str()).expect("set up");
            assert_eq!(
                done,
                Done::Changed {
                    backup: Some(files.backup())
                }
            );
            assert_eq!(
                std::fs::read_to_string(files.backup()).expect("backup"),
                "Host old\n"
            );
            let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode(&files.ours), 0o600);
            assert_eq!(mode(files.ours.parent().expect("dir")), 0o700);
            assert_eq!(
                setup(&files, "a", BLOCK_A, ssh.as_os_str()),
                Ok(Done::Nothing)
            );
            setup(&files, "b", BLOCK_B, ssh.as_os_str()).expect("second host");
            assert_eq!(remove(&files, "a"), Ok(Done::Changed { backup: None }));
            assert!(
                std::fs::read_to_string(&files.config)
                    .expect("read")
                    .contains(INCLUDE)
            );
            assert_eq!(remove(&files, "b"), Ok(Done::Changed { backup: None }));
            assert!(!files.ours.exists(), "the empty file goes");
            assert_eq!(
                std::fs::read_to_string(&files.config).expect("read"),
                "Host old\n"
            );
            assert_eq!(remove(&files, "b"), Ok(Done::Nothing));
            let _gone = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_block_ssh_refuses_writes_nothing() {
            let dir = scratch("refused");
            let files = Files::under(&dir.join(".ssh"));
            let ssh = fake_ssh(&dir, false);
            let refused = setup(&files, "a", BLOCK_A, ssh.as_os_str());
            assert!(refused.is_err_and(|e| e.contains("bad option")));
            assert!(!files.ours.exists() && !files.config.exists());
            let left: Vec<_> = std::fs::read_dir(files.ours.parent().expect("dir"))
                .expect("dir")
                .collect();
            assert!(left.is_empty(), "no temp file left");
            let _gone = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_symlinked_config_is_refused() {
            let dir = scratch("symlink");
            let files = Files::under(&dir.join(".ssh"));
            std::fs::create_dir_all(dir.join(".ssh")).expect("made");
            std::fs::write(dir.join("real"), "Host x\n").expect("written");
            std::os::unix::fs::symlink(dir.join("real"), &files.config).expect("linked");
            let ssh = fake_ssh(&dir, true);
            let refused = setup(&files, "a", BLOCK_A, ssh.as_os_str());
            assert!(refused.is_err_and(|e| e.contains("symlink")));
            assert!(!files.ours.exists());
            let _gone = std::fs::remove_dir_all(dir);
        }
    }
}
