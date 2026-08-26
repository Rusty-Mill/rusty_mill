//! vi-mode REPL, for pty tests and manual testing:
//!
//!     cargo run --example vi
//!
//! `Hooks::vi_mode` is on, so Esc enters normal mode (counts, operators,
//! text objects, `%`, `G` …); the mode indicator (`(ins)`/`(cmd)`) is
//! shown via `set_show_mode_in_prompt`, and the prompt is deliberately
//! multi-line to exercise the prefix-line handling. Ctrl-D on an empty
//! line exits.

use rusty_lines::{Editor, Hooks, ReadResult};

struct ViHooks;

impl Hooks for ViHooks {
    fn vi_mode(&self) -> bool {
        true
    }
}

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    ed.set_show_mode_in_prompt(true);
    loop {
        match ed.read_line("vi demo\nvi> ", "", &ViHooks)? {
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
