//! `rtouch` — change file timestamps or create empty file

use std::env;
use std::fs::OpenOptions;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let targets: Vec<&String> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();

    if targets.is_empty() {
        eprintln!("Usage: rtouch <file...>");
        std::process::exit(1);
    }

    for target in targets {
        let win_path = rpath::posix_to_win32(target);
        let path = Path::new(&win_path);

        // `truncate(false)` is explicit rather than incidental: `touch`
        // must never clobber an existing file's contents, so the one
        // behavior this open must not have is the one `create(true)`
        // leaves ambiguous (clippy::suspicious_open_options).
        match OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
        {
            Ok(_) => {}
            Err(e) => eprintln!("rtouch: cannot touch '{}': {}", target, e),
        }
    }

    Ok(())
}
