//! `rfind` — search for files in a directory hierarchy

use rusty_regx::Regex;
use std::env;
use std::fs;
use std::path::Path;

fn walk_dir(
    dir: &Path,
    name_regex: &Option<Regex>,
    type_filter: &Option<char>,
) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        let is_dir = path.is_dir();
        let matches_type = match type_filter {
            Some('d') => is_dir,
            Some('f') => !is_dir,
            _ => true,
        };

        let matches_name = match name_regex {
            Some(re) => re.is_match(&file_name),
            None => true,
        };

        if matches_type && matches_name {
            let posix_display = rpath::win32_to_posix(&path.to_string_lossy());
            println!("{}", posix_display);
        }

        if is_dir {
            let _ = walk_dir(&path, name_regex, type_filter);
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut search_path = ".".to_string();
    let mut name_pattern: Option<String> = None;
    let mut type_filter: Option<char> = None;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-name" && i + 1 < args.len() {
            name_pattern = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "-type" && i + 1 < args.len() {
            type_filter = args[i + 1].chars().next();
            i += 2;
        } else if !args[i].starts_with('-') && i == 1 {
            search_path = args[i].clone();
            i += 1;
        } else {
            i += 1;
        }
    }

    let name_regex = if let Some(pattern) = name_pattern {
        // Convert simple wildcard * pattern to regex if needed
        let regex_str = pattern.replace('.', "\\.").replace('*', ".*");
        Some(Regex::new(&format!("^{}$", regex_str))?)
    } else {
        None
    };

    let win_path = rpath::posix_to_win32(&search_path);
    let root = Path::new(&win_path);

    let is_dir = root.is_dir();
    let matches_type = match type_filter {
        Some('d') => is_dir,
        Some('f') => !is_dir,
        _ => true,
    };
    if matches_type {
        println!("{}", rpath::win32_to_posix(&root.to_string_lossy()));
    }

    if is_dir {
        walk_dir(root, &name_regex, &type_filter)?;
    }

    Ok(())
}
