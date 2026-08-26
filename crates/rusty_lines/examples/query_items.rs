//! `completion_query_items` demo, for the pty test: three candidates
//! sharing no common prefix (so Tab lists immediately) with the
//! "Display all N possibilities?" threshold set low enough to trigger:
//!
//!     cargo run --example query_items

use rusty_lines::{Candidate, Editor, Hooks, ReadResult};

struct QueryItemsHooks;

impl Hooks for QueryItemsHooks {
    fn complete(&self, _line: &str, _pos: usize) -> (usize, Vec<Candidate>) {
        let cand = |s: &str| Candidate {
            display: s.to_string(),
            replacement: s.to_string(),
        };
        (0, vec![cand("alpha"), cand("beta"), cand("gamma")])
    }
}

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    ed.set_completion_query_items(2);
    loop {
        match ed.read_line("qi> ", "", &QueryItemsHooks)? {
            ReadResult::Line(line) => println!("{line}"),
            ReadResult::Interrupted => println!("^C"),
            ReadResult::Eof | ReadResult::TimedOut => break,
        }
    }
    Ok(())
}
