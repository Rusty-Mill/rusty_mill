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

pub(crate) fn show_diff(diff: &[DiffLine], out: &mut impl Write) -> io::Result<()> {
    for line in diff {
        match line {
            DiffLine::Same(t) => writeln!(out, "  {t}")?,
            DiffLine::Removed(t) => writeln!(out, "- {t}")?,
            DiffLine::Added(t) => writeln!(out, "+ {t}")?,
        }
    }
    Ok(())
}

fn show(preview: &EditPreview, out: &mut impl Write) -> io::Result<()> {
    show_diff(&preview.diff, out)?;
    if preview.marks_edited {
        writeln!(out, "note: this is a copy; saving leaves it \"edited\"")?;
    }
    Ok(())
}

fn other(msg: impl Into<String>) -> Error {
    Error::Io(io::Error::other(msg.into()))
}

/// Runs the editor on a private temp file holding `old`, returns the result.
fn from_editor(cmd: &str, name: &str, old: &str) -> Result<String, Error> {
    let mut words = cmd.split_whitespace();
    let prog = words.next().ok_or_else(|| other("empty editor command"))?;
    let ext = Path::new(name).extension().and_then(|e| e.to_str());
    let dir = std::env::temp_dir().join(format!("aienv-edit-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let file: PathBuf = dir.join(format!("edit.{}", ext.unwrap_or("txt")));
    let run = || -> Result<String, Error> {
        fs::write(&file, old)?;
        let status = Command::new(prog).args(words).arg(&file).status()?;
        if !status.success() {
            return Err(other(format!("editor {prog:?} exited with {status}")));
        }
        Ok(fs::read_to_string(&file)?)
    };
    let result = run();
    let _ = fs::remove_dir_all(&dir);
    result
}

fn new_text(c: &EditFile, old: &str, s: &mut Session) -> Result<String, Error> {
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
            from_editor(&cmd, &c.file, old)
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
    let text = new_text(c, &opened.text, s)?;
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
