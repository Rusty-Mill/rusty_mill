//! `rrm` — remove files or directories

use std::env;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let recursive = args.contains(&"-r".to_string())
        || args.contains(&"-R".to_string())
        || args.contains(&"-rf".to_string());
    let force = args.contains(&"-f".to_string()) || args.contains(&"-rf".to_string());

    let targets: Vec<&String> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();

    if targets.is_empty() {
        if !force {
            eprintln!("Usage: rrm [-r] [-f] <file/dir...>");
            std::process::exit(1);
        }
        return Ok(());
    }

    for target in targets {
        let target_win = rpath::posix_to_win32(target);
        let path = Path::new(&target_win);

        if !path.exists() {
            if !force {
                eprintln!("rrm: cannot remove '{}': No such file or directory", target);
            }
            continue;
        }

        if path.is_dir() {
            if !recursive {
                eprintln!("rrm: cannot remove '{}': Is a directory", target);
                continue;
            }
            if let Err(e) = fs::remove_dir_all(path) {
                if !force {
                    eprintln!("rrm: cannot remove directory '{}': {}", target, e);
                }
            }
        } else {
            if let Err(e) = fs::remove_file(path) {
                if !force {
                    eprintln!("rrm: cannot remove file '{}': {}", target, e);
                }
            }
        }
    }

    Ok(())
}
