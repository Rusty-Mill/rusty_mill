//! `rwc` — print newline, word, and byte counts for files

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

struct Counts {
    lines: usize,
    words: usize,
    bytes: usize,
}

fn count_reader<R: Read>(mut reader: R) -> io::Result<Counts> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;

    let bytes = buf.len();
    let text = String::from_utf8_lossy(&buf);
    let lines = text.lines().count();
    let words = text.split_whitespace().count();

    Ok(Counts {
        lines,
        words,
        bytes,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let count_lines = args.contains(&"-l".to_string());
    let count_words = args.contains(&"-w".to_string());
    let count_bytes = args.contains(&"-c".to_string()) || args.contains(&"-m".to_string());

    // Default to all if no specific flags
    let (do_l, do_w, do_c) = if !count_lines && !count_words && !count_bytes {
        (true, true, true)
    } else {
        (count_lines, count_words, count_bytes)
    };

    let files: Vec<&String> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();

    if files.is_empty() {
        let counts = count_reader(io::stdin())?;
        print_counts(&counts, do_l, do_w, do_c, "");
        return Ok(());
    }

    let mut total = Counts {
        lines: 0,
        words: 0,
        bytes: 0,
    };
    for f in &files {
        let win_path = rpath::posix_to_win32(f);
        let path = Path::new(&win_path);
        match fs::File::open(path) {
            Ok(file) => {
                let counts = count_reader(file)?;
                total.lines += counts.lines;
                total.words += counts.words;
                total.bytes += counts.bytes;
                print_counts(&counts, do_l, do_w, do_c, f);
            }
            Err(e) => eprintln!("rwc: {}: {}", f, e),
        }
    }

    if files.len() > 1 {
        print_counts(&total, do_l, do_w, do_c, "total");
    }

    Ok(())
}

fn print_counts(counts: &Counts, l: bool, w: bool, c: bool, name: &str) {
    let mut out = String::new();
    if l {
        out.push_str(&format!("{:8} ", counts.lines));
    }
    if w {
        out.push_str(&format!("{:8} ", counts.words));
    }
    if c {
        out.push_str(&format!("{:8} ", counts.bytes));
    }
    if !name.is_empty() {
        out.push_str(name);
    }
    println!("{}", out.trim_end());
}
