//! `mill-term` — MSYS2 & Git Bash replacement environment launcher for Rusty Mill.

use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Rusty Mill Environment Launcher ===");
    println!("Powered by rush, rustils, rpath, SHH, rusty_git & rusty_term");
    println!("Type 'help' or commands to interact with the environment.\n");

    let current_dir = std::env::current_dir()?;
    let posix_cwd = rpath::win32_to_posix(&current_dir.to_string_lossy());
    println!("Workspace POSIX Path: {}", posix_cwd);

    let status = Command::new("rush")
        .status();

    match status {
        Ok(s) => std::process::exit(s.code().unwrap_or(0)),
        Err(_) => {
            eprintln!("Launching embedded rush session...");
            // Run rush from target if installed
            let rush_exe = current_dir.join("target").join("debug").join("rush.exe");
            if rush_exe.exists() {
                let s = Command::new(rush_exe).status()?;
                std::process::exit(s.code().unwrap_or(0));
            } else {
                eprintln!("Please build the workspace using 'cargo build --workspace'.");
            }
        }
    }

    Ok(())
}
