//! `rawk` — pattern scanning and text processing language

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use rusty_text::AwkProcessor;

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
        } else if args[i].starts_with("-F") {
            field_sep = args[i][2..].to_string();
            i += 1;
        } else if !args[i].starts_with('-') {
            if script.is_none() {
                script = Some(args[i].clone());
            } else {
                files.push(&args[i]);
            }
            i += 1;
        } else {
            i += 1;
        }
    }

    let script_str = match script {
        Some(s) => s,
        None => {
            eprintln!("Usage: rawk [-F fs] '{{print $1, $3}}' [file...]");
            std::process::exit(1);
        }
    };

    // Parse requested fields: e.g. '{print $1, $3}' -> [1, 3]
    let mut fields = Vec::new();
    for token in script_str.split(&[' ', ',', '{', '}', '\''][..]) {
        if let Some(num_str) = token.strip_prefix('$') {
            if let Ok(num) = num_str.parse::<usize>() {
                fields.push(num);
            }
        }
    }
    if fields.is_empty() {
        fields = vec![0]; // Default to printing entire line $0
    }

    let processor = AwkProcessor::new(&field_sep, fields);

    let process_reader = |reader: &mut dyn BufRead| -> io::Result<()> {
        for line in reader.lines() {
            let l = line?;
            let res = processor.process_line(&l);
            println!("{}", res);
        }
        Ok(())
    };

    if files.is_empty() {
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        process_reader(&mut reader)?;
    } else {
        for f in files {
            let win_path = rpath::posix_to_win32(f);
            let file = fs::File::open(Path::new(&win_path))?;
            let mut reader = BufReader::new(file);
            process_reader(&mut reader)?;
        }
    }

    Ok(())
}
