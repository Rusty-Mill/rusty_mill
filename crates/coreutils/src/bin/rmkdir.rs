//! `rmkdir` — make directories

use std::env;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let create_parents = args.contains(&"-p".to_string());
    let targets: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();

    if targets.is_empty() {
        eprintln!("Usage: rmkdir [-p] <directory...>");
        std::process::exit(1);
    }

    for target in targets {
        let win_path = rpath::posix_to_win32(target);
        let path = Path::new(&win_path);

        let res = if create_parents {
            fs::create_dir_all(path)
        } else {
            fs::create_dir(path)
        };

        if let Err(e) = res {
            eprintln!("rmkdir: cannot create directory '{}': {}", target, e);
        }
    }

    Ok(())
}
