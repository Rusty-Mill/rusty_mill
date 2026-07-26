//! `rgrep` — print lines matching a POSIX-ERE pattern using `rusty_regx`

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use rusty_regx::Regex;

fn search_reader<R: BufRead>(
    reader: R,
    re: &Regex,
    invert: bool,
    line_nums: bool,
    filename: &str,
    print_name: bool,
) -> io::Result<()> {
    for (idx, line) in reader.lines().enumerate() {
        let l = line?;
        let is_match = re.is_match(&l);
        let show = if invert { !is_match } else { is_match };

        if show {
            let mut prefix = String::new();
            if print_name {
                prefix.push_str(&format!("{}:", filename));
            }
            if line_nums {
                prefix.push_str(&format!("{}:", idx + 1));
            }
            println!("{}{}", prefix, l);
        }
    }
    Ok(())
}

fn search_dir(
    dir: &Path,
    re: &Regex,
    invert: bool,
    line_nums: bool,
) -> io::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                search_dir(&path, re, invert, line_nums)?;
            } else {
                if let Ok(file) = fs::File::open(&path) {
                    let reader = BufReader::new(file);
                    let display_path = rpath::win32_to_posix(&path.to_string_lossy());
                    let _ = search_reader(reader, re, invert, line_nums, &display_path, true);
                }
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let ignore_case = args.contains(&"-i".to_string());
    let invert_match = args.contains(&"-v".to_string());
    let line_numbers = args.contains(&"-n".to_string());
    let recursive = args.contains(&"-r".to_string()) || args.contains(&"-R".to_string());

    let non_opts: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();

    if non_opts.is_empty() {
        eprintln!("Usage: rgrep [-i] [-v] [-n] [-r] PATTERN [FILE...]");
        std::process::exit(1);
    }

    let pattern_str = non_opts[0];
    let final_pattern = if ignore_case {
        // Convert to lowercase regex pattern if -i flag
        pattern_str.to_lowercase()
    } else {
        pattern_str.to_string()
    };

    let re = Regex::new(&final_pattern).map_err(|e| format!("Invalid regex '{}': {:?}", pattern_str, e))?;
    let files = &non_opts[1..];

    if recursive && files.is_empty() {
        let root = Path::new(".");
        search_dir(root, &re, invert_match, line_numbers)?;
        return Ok(());
    }

    if files.is_empty() {
        let stdin = io::stdin();
        search_reader(stdin.lock(), &re, invert_match, line_numbers, "", false)?;
        return Ok(());
    }

    let print_name = files.len() > 1 || recursive;

    for f in files {
        let win_path = rpath::posix_to_win32(f);
        let path = Path::new(&win_path);

        if path.is_dir() {
            if recursive {
                search_dir(path, &re, invert_match, line_numbers)?;
            } else {
                eprintln!("rgrep: {}: Is a directory", f);
            }
        } else {
            match fs::File::open(path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    search_reader(reader, &re, invert_match, line_numbers, f, print_name)?;
                }
                Err(e) => eprintln!("rgrep: {}: {}", f, e),
            }
        }
    }

    Ok(())
}
