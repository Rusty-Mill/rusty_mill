//! `rusty_text`: a real (subset, honestly-documented) sed engine and awk
//! engine, both built on `rusty_regx`. See [`sed`] and [`awk`]'s own module
//! docs for exactly what's implemented vs. deliberately out of scope.

use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

pub mod awk;
pub mod sed;

pub use awk::AwkProgram;
pub use sed::SedScript;

/// Read input lines for `rsed`/`rawk`: stdin when `files` is empty,
/// otherwise each file in turn (translated through [`rpath::posix_to_win32`]
/// first).
pub fn read_lines(files: &[String]) -> io::Result<Vec<String>> {
    let mut lines = Vec::new();
    if files.is_empty() {
        for line in io::stdin().lock().lines() {
            lines.push(line?);
        }
    } else {
        for f in files {
            let win_path = rpath::posix_to_win32(f);
            let file = fs::File::open(Path::new(&win_path))?;
            for line in BufReader::new(file).lines() {
                lines.push(line?);
            }
        }
    }
    Ok(lines)
}
