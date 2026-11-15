//! `aienv push`: copy the edits in a tool's marked copy back to the skill's
//! source, after showing the diff. Same consent rule as `edit` (D6, D8, D42).

use crate::args::Push;
use crate::edit::show_diff;
use crate::run::{entries, Error, Outcome};
use ai_env_app::{find_entry, push_apply, push_preview, select_tools, Delta, Difference};
use std::io::Write;
use std::path::Path;

fn describe(d: &Difference) -> &'static str {
    match d.delta {
        Delta::Differs => "differs (not selected)",
        Delta::OnlyInCopy => "only in the copy (a push never creates files)",
        Delta::OnlyInSource => "only in the source (a push never deletes files)",
        Delta::KindDiffers => "a file on one side, a folder on the other",
    }
}

fn list_left(left: &[Difference], out: &mut impl Write) -> std::io::Result<()> {
    for d in left {
        writeln!(out, "  left    {} {}", d.path, describe(d))?;
    }
    Ok(())
}

pub(crate) fn push(
    c: &Push,
    home: &Path,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Outcome, Error> {
    let tools = select_tools(&c.scope, home)?;
    let entries = entries(&c.scope, err)?;
    let entry = find_entry(&entries, &c.entry)?;
    let preview = push_preview(entry, &c.tool, &tools, c.file.as_deref())?;
    for f in &preview.files {
        writeln!(out, "{}:", f.base.path)?;
        show_diff(&f.diff, out)?;
    }
    list_left(&preview.left, out)?;
    writeln!(
        out,
        "note: the source is overwritten, including changes made there since the copy was taken"
    )?;
    if !c.yes {
        writeln!(out, "dry run: nothing changed. Add --yes to push.")?;
        return Ok(Outcome::Done);
    }
    let bases: Vec<_> = preview.files.into_iter().map(|f| f.base).collect();
    let done = push_apply(entry, &c.tool, &tools, &bases)?;
    writeln!(out, "pushed {}", done.written.join(", "))?;
    if done.remarked {
        writeln!(out, "the copy matches the source again")?;
    } else {
        writeln!(out, "the copy is still edited; these differ:")?;
        list_left(&done.left, out)?;
    }
    Ok(Outcome::Done)
}
