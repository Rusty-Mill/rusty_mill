//! Minimal REPL showing plain editing with no hooks:
//!
//!     cargo run --example demo
//!
//! Type lines (full emacs keymap, C-r search, kill ring, undo…); each is
//! echoed back and added to history. Ctrl-D on an empty line exits.

use rusty_lines::{Editor, NoHooks, ReadResult};

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    loop {
        match ed.read_line("demo> ", "", &NoHooks)? {
            ReadResult::Line(line) => {
                if !line.trim().is_empty() {
                    ed.add_history_entry(&line);
                }
                println!("{line}");
            }
            ReadResult::Interrupted => println!("^C"),
            ReadResult::Eof | ReadResult::TimedOut => break,
        }
    }
    Ok(())
}
