//! Minimal reedline REPL for the pty benchmark.
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};

fn main() -> std::io::Result<()> {
    let mut ed = Reedline::create();
    let prompt = DefaultPrompt::new(
        DefaultPromptSegment::Basic("b>".to_string()),
        DefaultPromptSegment::Empty,
    );
    loop {
        match ed.read_line(&prompt) {
            Ok(Signal::Success(line)) => println!("{line}"),
            Ok(Signal::CtrlC) => println!("^C"),
            Ok(Signal::CtrlD) | Err(_) => break,
        }
    }
    Ok(())
}
