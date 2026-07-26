//! `rtouch` — change file timestamps or create empty file

use std::env;
use std::fs::OpenOptions;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let targets: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();

    if targets.is_empty() {
        eprintln!("Usage: rtouch <file...>");
        std::process::exit(1);
    }

    for target in targets {
        let win_path = rpath::posix_to_win32(target);
        let path = Path::new(&win_path);

        match OpenOptions::new().write(true).create(true).open(path) {
            Ok(_) => {},
            Err(e) => eprintln!("rtouch: cannot touch '{}': {}", target, e),
        }
    }

    Ok(())
}
