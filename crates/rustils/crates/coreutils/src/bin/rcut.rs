//! `rcut` — remove sections from each line of files

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut delimiter = "\t".to_string();
    let mut fields_str = String::new();
    let mut files = Vec::new();

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-d" && i + 1 < args.len() {
            delimiter = args[i + 1].clone();
            i += 2;
        } else if args[i].starts_with("-d") {
            delimiter = args[i][2..].to_string();
            i += 1;
        } else if args[i] == "-f" && i + 1 < args.len() {
            fields_str = args[i + 1].clone();
            i += 2;
        } else if args[i].starts_with("-f") {
            fields_str = args[i][2..].to_string();
            i += 1;
        } else if !args[i].starts_with('-') {
            files.push(&args[i]);
            i += 1;
        } else {
            i += 1;
        }
    }

    let selected_indices: Vec<usize> = fields_str
        .split(',')
        .filter_map(|s| s.parse::<usize>().ok())
        .map(|idx| if idx > 0 { idx - 1 } else { 0 })
        .collect();

    let process_line = |line: &str| {
        if selected_indices.is_empty() {
            println!("{}", line);
            return;
        }
        let parts: Vec<&str> = line.split(&delimiter).collect();
        let selected_parts: Vec<&str> = selected_indices
            .iter()
            .filter_map(|&idx| parts.get(idx).copied())
            .collect();
        println!("{}", selected_parts.join(&delimiter));
    };

    if files.is_empty() {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            process_line(&line?);
        }
    } else {
        for f in files {
            let win_path = rpath::posix_to_win32(f);
            let file = fs::File::open(Path::new(&win_path))?;
            let reader = BufReader::new(file);
            for line in reader.lines() {
                process_line(&line?);
            }
        }
    }

    Ok(())
}
