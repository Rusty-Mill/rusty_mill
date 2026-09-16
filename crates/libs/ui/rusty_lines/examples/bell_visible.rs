//! `BellStyle::Visible` demo, for the pty test proving the reverse-video
//! flash sequence (`ESC[?5h` / `ESC[?5l`) is what actually gets written —
//! Up-arrow before anything is typed rings the "nothing to recall" bell:
//!
//!     cargo run --example bell_visible

use rusty_lines::{BellStyle, Editor, NoHooks, ReadResult};

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    ed.set_bell_style(BellStyle::Visible);
    loop {
        match ed.read_line("bell> ", "", &NoHooks)? {
            ReadResult::Line(line) => println!("{line}"),
            ReadResult::Interrupted => println!("^C"),
            ReadResult::Eof | ReadResult::TimedOut => break,
        }
    }
    Ok(())
}
