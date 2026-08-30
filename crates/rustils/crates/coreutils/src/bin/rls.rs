//! Thin CLI over `coreutils::ls`, wired to the real backend for this OS.
//! Like `rcat`, exists to prove the full stack end to end; the listing
//! logic is tested against platform-mock in the library.
//!
//! `-l` switches to the long format (`coreutils::ls::ls_long`/
//! `format_long`, coreutils gap backlog #65) — this is the one place
//! `SystemTime::now()` and real uid/gid name resolution
//! (`coreutils::native::user_name`/`group_name`) enter the picture;
//! the library itself stays deterministic and OS-independent.

use std::path::Path;

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let long = args.iter().any(|a| a == "-l");
    let arg = args
        .into_iter()
        .find(|a| a != "-l")
        .unwrap_or_else(|| std::ffi::OsString::from("."));
    let dir = match coreutils::native::open_dir(Path::new(&arg)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("rls: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if long {
        let entries = match coreutils::ls::ls_long(dir.as_ref()) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("rls: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        let lines = coreutils::ls::format_long(
            &entries,
            std::time::SystemTime::now(),
            coreutils::native::user_name,
            coreutils::native::group_name,
        );
        for line in lines {
            println!("{line}");
        }
        return std::process::ExitCode::SUCCESS;
    }
    match coreutils::ls::ls(dir.as_ref()) {
        Ok(entries) => {
            for entry in &entries {
                println!("{}", coreutils::ls::render(entry));
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("rls: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
