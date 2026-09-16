//! REPL with a read deadline, for trying (and pty-testing)
//! `read_line_timeout` — a shell's `$TMOUT` idle auto-logout:
//!
//!     cargo run --example timeout
//!
//! Each read times out after two seconds of no complete line; the demo
//! then prints bash's message and exits.

use rusty_lines::{Editor, NoHooks, ReadResult};
use std::time::Duration;

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    loop {
        match ed.read_line_timeout("timeout> ", "", &NoHooks, Some(Duration::from_secs(2)))? {
            ReadResult::Line(line) => {
                if !line.trim().is_empty() {
                    ed.add_history_entry(&line);
                }
                println!("{line}");
            }
            ReadResult::Interrupted => println!("^C"),
            ReadResult::Eof => break,
            ReadResult::TimedOut => {
                println!("timed out waiting for input");
                break;
            }
        }
    }
    Ok(())
}
