//! `aienv edit`: change one text file of a skill. The new text comes from a
//! file, stdin or the user's editor; the diff is shown and the save needs
//! consent (`--yes`, or a typed `y` on a terminal), the CLI form of the GUI's
//! Confirm (D6, D8). All rules live in `ai_env_app`.

use crate::args::{EditFile, NewText};
use crate::run::{entries, Error, Outcome};
use ai_env_app::{
    edit_apply, edit_preview, find_entry, read_file, select_tools, DiffLine, EditPreview, Place,
};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// How `edit` reaches the person: the input stream, whether it is a terminal,
/// and the editor command (`$VISUAL`, else `$EDITOR`).
pub struct Session<'a> {
    input: &'a mut dyn BufRead,
    interactive: bool,
    editor: Option<String>,
}

impl<'a> Session<'a> {
    pub fn new(input: &'a mut dyn BufRead, interactive: bool, editor: Option<String>) -> Self {
        Self {
            input,
            interactive,
            editor,
        }
    }

    /// No terminal, no editor: for commands that never ask.
    pub fn none(input: &'a mut dyn BufRead) -> Self {
        Self::new(input, false, None)
    }
}

fn show(preview: &EditPreview, out: &mut impl Write) -> io::Result<()> {
    for line in &preview.diff {
        match line {
            DiffLine::Same(t) => writeln!(out, "  {t}")?,
            DiffLine::Removed(t) => writeln!(out, "- {t}")?,
            DiffLine::Added(t) => writeln!(out, "+ {t}")?,
        }
    }
    if preview.marks_edited {
        writeln!(out, "note: this is a copy; saving leaves it \"edited\"")?;
    }
    Ok(())
}

fn other(msg: impl Into<String>) -> Error {
    Error::Io(io::Error::other(msg.into()))
}

/// A new, empty folder only this user can enter. The name is not guessable
/// and an existing path is never reused, so nobody can plant a link there.
fn private_dir() -> io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU32 = AtomicU32::new(0);
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    for _ in 0..16 {
        let dir = std::env::temp_dir().join(format!(
            "aienv-edit-{}-{nanos:x}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a fresh temporary folder",
    ))
}

/// Runs the editor on a file in a private temp folder holding `old`, returns
/// the result. A folder that cannot be removed afterwards is reported on `err`.
fn from_editor(cmd: &str, name: &str, old: &str, err: &mut impl Write) -> Result<String, Error> {
    let mut words = cmd.split_whitespace();
    let prog = words.next().ok_or_else(|| other("empty editor command"))?;
    let ext = Path::new(name).extension().and_then(|e| e.to_str());
    let dir = private_dir()?;
    let file: PathBuf = dir.join(format!("edit.{}", ext.unwrap_or("txt")));
    let run = || -> Result<String, Error> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)?;
        f.write_all(old.as_bytes())?;
        drop(f);
        let status = Command::new(prog).args(words).arg(&file).status()?;
        if !status.success() {
            return Err(other(format!("editor {prog:?} exited with {status}")));
        }
        Ok(fs::read_to_string(&file)?)
    };
    let result = run();
    if let Err(e) = fs::remove_dir_all(&dir) {
        writeln!(err, "aienv: could not remove {}: {e}", dir.display())?;
    }
    result
}

fn new_text(
    c: &EditFile,
    old: &str,
    s: &mut Session,
    err: &mut impl Write,
) -> Result<String, Error> {
    match &c.text {
        NewText::File(p) => Ok(fs::read_to_string(p)?),
        NewText::Stdin => {
            let mut text = String::new();
            s.input.read_to_string(&mut text)?;
            Ok(text)
        }
        NewText::Editor => {
            let cmd = s.editor.clone().ok_or_else(|| {
                other("no editor: set $VISUAL or $EDITOR, or pass --from PATH (or - for stdin)")
            })?;
            from_editor(&cmd, &c.file, old, err)
        }
    }
}

/// Asks "Save? [y/N]"; false on anything but y/yes.
fn confirm(s: &mut Session, out: &mut impl Write) -> Result<bool, Error> {
    write!(out, "Save? [y/N] ")?;
    out.flush()?;
    let mut answer = String::new();
    s.input.read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

pub(crate) fn edit(
    c: &EditFile,
    home: &Path,
    s: &mut Session,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Outcome, Error> {
    let tools = select_tools(&c.scope, home)?;
    let entries = entries(&c.scope, err)?;
    let entry = find_entry(&entries, &c.entry)?;
    let place = c
        .scope
        .tools
        .first()
        .map_or(Place::Source, |t| Place::Tool(t.clone()));
    let opened = read_file(entry, &place, &tools, &c.file)?;
    // The editor is the one mode that needs a person; refuse before opening
    // it if the save could not be confirmed.
    let editing = c.text == NewText::Editor;
    if editing && !c.yes && !s.interactive {
        writeln!(
            err,
            "aienv: the editor needs a terminal to confirm; pass --yes"
        )?;
        return Ok(Outcome::NeedsTerminal);
    }
    let text = new_text(c, &opened.text, s, err)?;
    let preview = match edit_preview(entry, &place, &tools, &c.file, text.clone(), &opened.hash) {
        Ok(p) => p,
        Err(Error::Edit(ai_env_core::EditRefusal::NoChange)) => {
            writeln!(out, "nothing changed")?;
            return Ok(Outcome::Done);
        }
        Err(e) => return Err(e),
    };
    show(&preview, out)?;
    let go = c.yes || (editing && confirm(s, out)?);
    if !go {
        writeln!(out, "dry run: nothing changed. Add --yes to save.")?;
        return Ok(Outcome::Done);
    }
    edit_apply(entry, &place, &tools, &c.file, text, &opened.hash)?;
    writeln!(out, "saved {}", c.file)?;
    Ok(Outcome::Done)
}
