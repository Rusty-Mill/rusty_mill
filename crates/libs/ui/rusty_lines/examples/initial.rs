//! `read_line_with_initial` demo, for the pty test: one line pre-seeded
//! with `"hello "` before the cursor and `"world"` after it, echoed back,
//! then exit.

use rusty_lines::{Editor, NoHooks, ReadResult};

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    if let ReadResult::Line(line) =
        ed.read_line_with_initial("init> ", "", &NoHooks, ("hello ", "world"))?
    {
        println!("{line}");
    }
    Ok(())
}
