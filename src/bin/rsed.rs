//! `rsed` — stream editor for filtering and transforming text

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use rusty_text::SedSubst;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut expr: Option<String> = None;
    let mut files = Vec::new();

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-e" && i + 1 < args.len() {
            expr = Some(args[i + 1].clone());
            i += 2;
        } else if !args[i].starts_with('-') {
            if expr.is_none() {
                expr = Some(args[i].clone());
            } else {
                files.push(&args[i]);
            }
            i += 1;
        } else {
            i += 1;
        }
    }

    let expr_str = match expr {
        Some(e) => e,
        None => {
            eprintln!("Usage: rsed [-e] 's/pattern/replacement/g' [file...]");
            std::process::exit(1);
        }
    };

    let sed_subst = SedSubst::parse(&expr_str)?;

    let process_reader = |reader: &mut dyn BufRead| -> io::Result<()> {
        for line in reader.lines() {
            let l = line?;
            let transformed = sed_subst.apply(&l);
            println!("{}", transformed);
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
