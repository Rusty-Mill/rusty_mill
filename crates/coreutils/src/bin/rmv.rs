//! `rmv` — move (rename) files

use std::env;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let paths: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();

    if paths.len() < 2 {
        eprintln!("Usage: rmv <source...> <destination>");
        std::process::exit(1);
    }

    let dst_str = paths.last().unwrap();
    let dst_win = rpath::posix_to_win32(dst_str);
    let dst_path = Path::new(&dst_win);

    let src_paths = &paths[..paths.len() - 1];

    if src_paths.len() > 1 && !dst_path.is_dir() {
        eprintln!("rmv: target '{}' is not a directory", dst_str);
        std::process::exit(1);
    }

    for src_str in src_paths {
        let src_win = rpath::posix_to_win32(src_str);
        let src_path = Path::new(&src_win);

        if !src_path.exists() {
            eprintln!("rmv: cannot stat '{}': No such file or directory", src_str);
            continue;
        }

        let final_dst = if dst_path.is_dir() {
            dst_path.join(src_path.file_name().unwrap())
        } else {
            dst_path.to_path_buf()
        };

        if let Err(_e) = fs::rename(src_path, &final_dst) {
            // Fallback to copy + remove if rename across file systems fails
            if let Err(copy_err) = fs::copy(src_path, &final_dst) {
                eprintln!("rmv: cannot move '{}' to '{}': {}", src_str, final_dst.display(), copy_err);
            } else {
                let _ = fs::remove_file(src_path);
            }
        }
    }

    Ok(())
}
