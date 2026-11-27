//! Measures every Rust and Python file under a directory and prints one
//! tab-separated line (card #705 Phase A):
//! `loc total_cc functions conc_num conc_den dup_num dup_den unmeasured`.
//!
//! ```text
//! cargo run -q -p rusty_erosion --example measure_tree -- <dir>
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rusty_erosion::{Lang, measure_in};

fn main() -> ExitCode {
    let Some(root) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: measure_tree <dir>");
        return ExitCode::from(2);
    };
    match run(&root) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("measure_tree: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.sort();
    let pairs = files.iter().map(|(p, s)| (p.as_str(), s.as_str()));
    let r = measure_in(&[Lang::Rust, Lang::Python], pairs).map_err(|e| e.to_string())?;
    Ok(format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        r.loc,
        r.total_cc,
        r.functions.len(),
        r.concentration.num,
        r.concentration.den,
        r.duplication.num,
        r.duplication.den,
        r.unmeasured.len(),
    ))
}

/// Reads every UTF-8 file under `dir` as a `/`-separated path relative to
/// `root`. Hidden entries and `__pycache__` are skipped.
fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.starts_with('.') || name == "__pycache__" {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out)?;
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
        let rel: Vec<_> = rel.iter().filter_map(|c| c.to_str()).collect();
        out.push((rel.join("/"), source));
    }
    Ok(())
}
