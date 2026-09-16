//! Minimal repro for `prepare_external_output`: watches for a trigger
//! file and, once it appears, prints a notice from `on_interrupted_read`'s
//! idle tick — for `tests/pty.rs` to prove the terminal repaints cleanly
//! around output printed outside the editor's own rendering, the same way
//! `demo`'s own idle-resize handling already proves the self-heal path
//! keeps an in-progress line intact.
//!
//!     cargo run --example notify -- <trigger-file-path>

use rusty_lines::{Editor, Hooks, ReadResult};
use std::cell::Cell;
use std::path::PathBuf;

struct NotifyHooks {
    trigger: PathBuf,
    fired: Cell<bool>,
}

impl Hooks for NotifyHooks {
    fn on_interrupted_read(&self) {
        if !self.fired.get() && self.trigger.exists() {
            self.fired.set(true);
            let _ = rusty_lines::prepare_external_output();
            println!("[1]+  Done\tsleep 5");
        }
    }
}

fn main() -> std::io::Result<()> {
    let trigger: PathBuf = std::env::args()
        .nth(1)
        .expect("usage: notify <trigger-file-path>")
        .into();
    let hooks = NotifyHooks {
        trigger,
        fired: Cell::new(false),
    };
    let mut ed = Editor::new();
    loop {
        match ed.read_line("notify> ", "", &hooks)? {
            ReadResult::Line(line) => println!("{line}"),
            ReadResult::Interrupted => println!("^C"),
            ReadResult::Eof | ReadResult::TimedOut => break,
        }
    }
    Ok(())
}
