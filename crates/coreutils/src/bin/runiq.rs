//! `runiq` — report or omit repeated lines

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let count = args.contains(&"-c".to_string());
    let duplicates_only = args.contains(&"-d".to_string());
    let unique_only = args.contains(&"-u".to_string());

    let files: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
    let mut lines = Vec::new();

    if files.is_empty() {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            lines.push(line?);
        }
    } else {
        let win_path = rpath::posix_to_win32(files[0]);
        let file = fs::File::open(Path::new(&win_path))?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            lines.push(line?);
        }
    }

    if lines.is_empty() {
        return Ok(());
    }

    let mut prev = &lines[0];
    let mut cur_count = 1;

    for line in lines.iter().skip(1) {
        if line == prev {
            cur_count += 1;
        } else {
            print_uniq(prev, cur_count, count, duplicates_only, unique_only);
            prev = line;
            cur_count = 1;
        }
    }
    print_uniq(prev, cur_count, count, duplicates_only, unique_only);

    Ok(())
}

fn print_uniq(line: &str, cur_count: usize, count: bool, duplicates_only: bool, unique_only: bool) {
    if duplicates_only && cur_count == 1 {
        return;
    }
    if unique_only && cur_count > 1 {
        return;
    }

    if count {
        println!("{:7} {}", cur_count, line);
    } else {
        println!("{}", line);
    }
}
