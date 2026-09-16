//! Right-side prompt demo, for the pty tests: one line read with an
//! rprompt (zsh's `$RPS1`), echoed back, then exit. The rprompt is shown
//! while the first row has room and hides once the line grows into it.

use rusty_lines::{Editor, NoHooks, ReadResult};

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    if let ReadResult::Line(line) = ed.read_line("rp> ", "RIGHT", &NoHooks)? {
        println!("{line}");
    }
    Ok(())
}
