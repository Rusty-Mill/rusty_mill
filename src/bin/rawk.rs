//! `rawk` — a real (subset) awk pattern-scanning/processing language. See
//! `rusty_text::awk`'s module doc for exactly what's implemented.

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use rusty_text::AwkProgram;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut field_sep = " ".to_string();
    let mut script: Option<String> = None;
    let mut files = Vec::new();

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-F" && i + 1 < args.len() {
            field_sep = args[i + 1].clone();
            i += 2;
        } else if let Some(fs) = args[i].strip_prefix("-F") {
            field_sep = fs.to_string();
            i += 1;
        } else if !args[i].starts_with('-') {
            if script.is_none() {
                script = Some(args[i].clone());
            } else {
                files.push(args[i].clone());
            }
            i += 1;
        } else {
            i += 1;
        }
    }

    let script_str = match script {
        Some(s) => s,
        None => {
            eprintln!("Usage: rawk [-F fs] 'BEGIN{{...}} pattern{{action}} END{{...}}' [file...]");
            std::process::exit(1);
        }
    };

    let program = AwkProgram::parse(&script_str)?;

    let mut lines: Vec<String> = Vec::new();
    if files.is_empty() {
        for line in io::stdin().lock().lines() {
            lines.push(line?);
        }
    } else {
        for f in &files {
            let win_path = rpath::posix_to_win32(f);
            let file = fs::File::open(Path::new(&win_path))?;
            for line in BufReader::new(file).lines() {
                lines.push(line?);
            }
        }
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    program.run(lines.iter().map(String::as_str), &field_sep, |l| {
        let _ = writeln!(out, "{l}");
    });

    Ok(())
}
