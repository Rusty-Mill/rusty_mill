//! Minimal rustyline REPL for the pty benchmark.
fn main() -> rustyline::Result<()> {
    let mut rl = rustyline::DefaultEditor::new()?;
    loop {
        match rl.readline("b> ") {
            Ok(line) => {
                let _ = rl.add_history_entry(line.as_str());
                println!("{line}");
            }
            Err(rustyline::error::ReadlineError::Interrupted) => println!("^C"),
            Err(_) => break,
        }
    }
    Ok(())
}
