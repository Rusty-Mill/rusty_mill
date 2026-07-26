//! `rcp` — copy files and directories

use std::env;
use std::fs;
use std::io;
use std::path::Path;

fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let child_src = entry.path();
            let child_dst = dst.join(entry.file_name());
            copy_recursive(&child_src, &child_dst)?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: rcp [-r] <source...> <destination>");
        std::process::exit(1);
    }

    let recursive = args.contains(&"-r".to_string()) || args.contains(&"-R".to_string());
    let paths: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();

    if paths.len() < 2 {
        eprintln!("rcp: missing destination file operand");
        std::process::exit(1);
    }

    let dst_str = paths.last().unwrap();
    let dst_win = rpath::posix_to_win32(dst_str);
    let dst_path = Path::new(&dst_win);

    let src_paths = &paths[..paths.len() - 1];

    if src_paths.len() > 1 && !dst_path.is_dir() {
        eprintln!("rcp: target '{}' is not a directory", dst_str);
        std::process::exit(1);
    }

    for src_str in src_paths {
        let src_win = rpath::posix_to_win32(src_str);
        let src_path = Path::new(&src_win);

        if !src_path.exists() {
            eprintln!("rcp: cannot stat '{}': No such file or directory", src_str);
            continue;
        }

        let final_dst = if dst_path.is_dir() {
            dst_path.join(src_path.file_name().unwrap())
        } else {
            dst_path.to_path_buf()
        };

        if src_path.is_dir() && !recursive {
            eprintln!("rcp: -r not specified; omitting directory '{}'", src_str);
            continue;
        }

        if let Err(e) = copy_recursive(src_path, &final_dst) {
            eprintln!("rcp: error copying '{}' to '{}': {}", src_str, final_dst.display(), e);
        }
    }

    Ok(())
}
