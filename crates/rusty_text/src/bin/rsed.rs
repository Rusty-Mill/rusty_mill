//! `rsed` — a real (subset) sed stream editor. See `rusty_text::sed`'s
//! module doc for exactly what's implemented.

use std::env;
use std::io;

use rusty_text::{read_lines, SedScript};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut script_parts: Vec<String> = Vec::new();
    let mut suppress_auto_print = false;
    let mut files = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-e" if i + 1 < args.len() => {
                script_parts.push(args[i + 1].clone());
                i += 2;
            }
            "-n" => {
                suppress_auto_print = true;
                i += 1;
            }
            arg if !arg.starts_with('-') => {
                if script_parts.is_empty() {
                    script_parts.push(arg.to_string());
                } else {
                    files.push(arg.to_string());
                }
                i += 1;
            }
            _ => i += 1,
        }
    }

    if script_parts.is_empty() {
        eprintln!("Usage: rsed [-n] [-e 'script']... ['script'] [file...]");
        eprintln!("Commands: s/pat/rep/[gpiN], d, p, q, =. Addresses: N, $, /re/, addr1,addr2.");
        std::process::exit(1);
    }

    let script = SedScript::parse(&script_parts.join(";"))?;

    let lines = read_lines(&files)?;

    let mut state = script.new_state();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    use std::io::Write;

    for (idx, line) in lines.iter().enumerate() {
        let is_last = idx + 1 == lines.len();
        let quit = script.run_line(&mut state, idx + 1, is_last, line, suppress_auto_print, |l| {
            let _ = writeln!(out, "{l}");
        });
        if quit {
            break;
        }
    }

    Ok(())
}
