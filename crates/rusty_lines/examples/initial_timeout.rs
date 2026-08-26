//! `read_line_with_initial_timeout` demo, for the pty test: seeds
//! "hello " / "world" like `initial`, but also carries a 2s deadline
//! like `timeout` — proving the two combine in one call.
//!
//!     cargo run --example initial_timeout

use rusty_lines::{Editor, NoHooks, ReadResult};
use std::time::Duration;

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    match ed.read_line_with_initial_timeout(
        "initial_timeout> ",
        "",
        &NoHooks,
        ("hello ", "world"),
        Some(Duration::from_secs(2)),
    )? {
        ReadResult::Line(line) => println!("{line}"),
        ReadResult::TimedOut => println!("timed out waiting for input"),
        ReadResult::Interrupted => println!("^C"),
        ReadResult::Eof => {}
    }
    Ok(())
}
