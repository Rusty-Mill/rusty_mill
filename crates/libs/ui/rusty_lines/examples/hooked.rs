//! REPL with hooks wired up, for trying (and pty-testing) completion,
//! menu cycling, and history hints:
//!
//!     cargo run --example hooked
//!
//! Tab completes from a fixed word list; typing shows a dimmed hint from
//! history (Right/End accepts it, M-f accepts one word); C-o is rebound
//! to unix-line-discard and C-g runs a host binding that uppercases the
//! line (bash `bind -x` style). C-x C-e hands the line to a fake
//! `$EDITOR` that deterministically overwrites it (for the pty test).
//! Ctrl-D on an empty line exits.

use rusty_lines::{Candidate, Editor, EditorAction, Hooks, ReadResult};

const WORDS: &[&str] = &["alpha", "alphabet", "alphanumeric", "beta", "gamma"];

struct DemoHooks;

impl Hooks for DemoHooks {
    fn complete(&self, line: &str, pos: usize) -> (usize, Vec<Candidate>) {
        let start = line[..pos].rfind(' ').map_or(0, |i| i + 1);
        let prefix = &line[start..pos];
        if prefix.is_empty() {
            return (0, Vec::new());
        }
        let candidates = WORDS
            .iter()
            .filter(|w| w.starts_with(prefix))
            .map(|w| Candidate {
                display: w.to_string(),
                replacement: w.to_string(),
            })
            .collect();
        (start, candidates)
    }

    fn hint(&self, line: &str, history: &[String]) -> Option<String> {
        if line.is_empty() {
            return None;
        }
        history
            .iter()
            .rev()
            .find(|h| h.starts_with(line) && h.len() > line.len())
            .map(|h| h[line.len()..].to_string())
    }

    fn host_binding(&self, tag: &str, line: &mut String, cursor: &mut usize) {
        if tag == "upcase" {
            *line = line.to_uppercase();
            *cursor = line.len();
        }
    }

    fn external_editor(&self) -> Option<String> {
        // Deterministic stand-in for $VISUAL/$EDITOR, for the pty test:
        // overwrites the tempfile instead of opening a real editor.
        Some("printf EDITED-VIA-EXTERNAL >".to_string())
    }
}

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    ed.bind("\\C-o", EditorAction::UnixLineDiscard)?;
    ed.bind_host("\\C-g", "upcase".to_string())?;
    loop {
        match ed.read_line("hooked> ", "", &DemoHooks)? {
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
