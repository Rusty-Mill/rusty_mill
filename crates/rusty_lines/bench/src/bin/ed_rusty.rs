//! Minimal rusty_lines REPL for the pty benchmark.
use rusty_lines::{Editor, NoHooks, ReadResult};

fn main() -> std::io::Result<()> {
    let mut ed = Editor::new();
    loop {
        match ed.read_line("b> ", "", &NoHooks)? {
            ReadResult::Line(line) => {
                if !line.is_empty() {
                    ed.add_history_entry(&line);
                }
                println!("{line}");
            }
            ReadResult::Interrupted => println!("^C"),
            _ => break,
        }
    }
    Ok(())
}
