//! `rhead` — output the first part of files

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut num_lines: usize = 10;
    let mut files = Vec::new();

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-n" && i + 1 < args.len() {
            num_lines = args[i + 1].parse().unwrap_or(10);
            i += 2;
        } else if args[i].starts_with("-n") {
            num_lines = args[i][2..].parse().unwrap_or(10);
            i += 1;
        } else if !args[i].starts_with('-') {
            files.push(&args[i]);
            i += 1;
        } else {
            i += 1;
        }
    }

    if files.is_empty() {
        let stdin = io::stdin();
        let reader = stdin.lock();
        for line in reader.lines().take(num_lines) {
            println!("{}", line?);
        }
        return Ok(());
    }

    for (idx, f) in files.iter().enumerate() {
        let win_path = rpath::posix_to_win32(f);
        let path = Path::new(&win_path);
        match fs::File::open(path) {
            Ok(file) => {
                if files.len() > 1 {
                    if idx > 0 { println!(); }
                    println!("==> {} <==", f);
                }
                let reader = BufReader::new(file);
                for line in reader.lines().take(num_lines) {
                    println!("{}", line?);
                }
            }
            Err(e) => eprintln!("rhead: {}: {}", f, e),
        }
    }

    Ok(())
}
