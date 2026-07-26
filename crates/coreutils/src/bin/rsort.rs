//! `rsort` — sort lines of text files

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let reverse = args.contains(&"-r".to_string());
    let numeric = args.contains(&"-n".to_string());

    let files: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
    let mut lines = Vec::new();

    if files.is_empty() {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            lines.push(line?);
        }
    } else {
        for f in files {
            let win_path = rpath::posix_to_win32(f);
            let path = Path::new(&win_path);
            let file = fs::File::open(path)?;
            let reader = BufReader::new(file);
            for line in reader.lines() {
                lines.push(line?);
            }
        }
    }

    if numeric {
        lines.sort_by(|a, b| {
            let num_a: f64 = a.trim().parse().unwrap_or(0.0);
            let num_b: f64 = b.trim().parse().unwrap_or(0.0);
            num_a.partial_cmp(&num_b).unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        lines.sort();
    }

    if reverse {
        lines.reverse();
    }

    for line in lines {
        println!("{}", line);
    }

    Ok(())
}
