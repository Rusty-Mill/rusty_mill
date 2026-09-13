//! `rcp` — copy files and directories

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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

/// Compute the destination path for a single source, given the already
/// resolved overall destination and whether it is a directory. Returns
/// `None` when `dst_is_dir` is true but `src_path` has no file-name
/// component (e.g. `.`, a path ending in `..`, or a bare drive root) —
/// the caller reports this per-source and skips it, matching the tool's
/// existing error-and-continue convention.
fn dest_for_source(dst_path: &Path, dst_is_dir: bool, src_path: &Path) -> Option<PathBuf> {
    if dst_is_dir {
        src_path.file_name().map(|name| dst_path.join(name))
    } else {
        Some(dst_path.to_path_buf())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = coreutils::args::collect_lossy(env::args_os());
    if args.len() < 3 {
        eprintln!("Usage: rcp [-r] <source...> <destination>");
        std::process::exit(1);
    }

    let recursive = args.contains(&"-r".to_string()) || args.contains(&"-R".to_string());
    let paths: Vec<&String> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();

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

    let mut exit_code: i32 = 0;
    for src_str in src_paths {
        let src_win = rpath::posix_to_win32(src_str);
        let src_path = Path::new(&src_win);

        if !src_path.exists() {
            eprintln!("rcp: cannot stat '{}': No such file or directory", src_str);
            continue;
        }

        let final_dst = match dest_for_source(dst_path, dst_path.is_dir(), src_path) {
            Some(p) => p,
            None => {
                eprintln!("rcp: cannot copy '{}': no file name", src_str);
                exit_code = 1;
                continue;
            }
        };

        if src_path.is_dir() && !recursive {
            eprintln!("rcp: -r not specified; omitting directory '{}'", src_str);
            continue;
        }

        if let Err(e) = copy_recursive(src_path, &final_dst) {
            eprintln!(
                "rcp: error copying '{}' to '{}': {}",
                src_str,
                final_dst.display(),
                e
            );
        }
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dest_for_source_joins_file_name_into_directory() {
        let dst = Path::new("/tmp/dest");
        let src = Path::new("/tmp/src/file.txt");
        assert_eq!(dest_for_source(dst, true, src), Some(dst.join("file.txt")));
    }

    #[test]
    fn dest_for_source_uses_dst_directly_when_not_a_directory() {
        let dst = Path::new("/tmp/dest.txt");
        let src = Path::new(".");
        assert_eq!(dest_for_source(dst, false, src), Some(dst.to_path_buf()));
    }

    #[test]
    fn dest_for_source_none_for_dot_source_into_directory() {
        let dst = Path::new("/tmp/dest");
        let src = Path::new(".");
        assert_eq!(dest_for_source(dst, true, src), None);
    }

    #[test]
    fn dest_for_source_none_for_trailing_dotdot_source_into_directory() {
        let dst = Path::new("/tmp/dest");
        let src = Path::new("foo/..");
        assert_eq!(dest_for_source(dst, true, src), None);
    }

    #[test]
    #[cfg(windows)]
    fn dest_for_source_none_for_bare_drive_root_into_directory() {
        // `Path::file_name()` on `C:\` returns `None` only under Windows
        // path-parsing rules (a bare drive root has no final component);
        // on Unix, backslash is not a separator, so `C:\` parses as an
        // ordinary (if unusual) file name instead.
        let dst = Path::new("/tmp/dest");
        let src = Path::new("C:\\");
        assert_eq!(dest_for_source(dst, true, src), None);
    }
}
