//! Thin CLI over `coreutils::cat`, wired to the real backend for this OS.
//! Exists to prove the full stack end to end (api → sys → ffi → kernel);
//! the logic itself is tested against platform-mock in the library.

use std::path::Path;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: rcat <file>");
        return std::process::ExitCode::from(2);
    };
    let p = Path::new(&path);
    let parent = p
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let Some(name) = p.file_name() else {
        eprintln!("rcat: not a file path: {}", p.display());
        return std::process::ExitCode::from(2);
    };
    let dir = match coreutils::native::open_dir(parent) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("rcat: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = coreutils::cat::cat(dir.as_ref(), name, &mut stdout) {
        eprintln!("rcat: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
