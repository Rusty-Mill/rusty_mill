//! Wide completion-candidate list, for the pty test proving
//! `print_candidate_columns` lays candidates out column-major (down each
//! column, then across) rather than row-major: eight same-width
//! candidates sharing no common prefix, so Tab lists them immediately:
//!
//!     cargo run --example columns

use rusty_lines::{Candidate, Editor, Hooks, ReadResult};

const WORDS: &[&str] = &[
    "a-marker-item-00",
    "b-marker-item-01",
    "c-marker-item-02",
    "d-marker-item-03",
    "e-marker-item-04",
    "f-marker-item-05",
    "g-marker-item-06",
    "h-marker-item-07",
];

struct ColumnsHooks;

impl Hooks for ColumnsHooks {
    fn complete(&self, _line: &str, _pos: usize) -> (usize, Vec<Candidate>) {
        let cand = |s: &str| Candidate {
            display: s.to_string(),
            replacement: s.to_string(),
        };
        (0, WORDS.iter().map(|w| cand(w)).collect())
    }
}

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    loop {
        match ed.read_line("cols> ", "", &ColumnsHooks)? {
            ReadResult::Line(line) => println!("{line}"),
            ReadResult::Interrupted => println!("^C"),
            ReadResult::Eof | ReadResult::TimedOut => break,
        }
    }
    Ok(())
}
