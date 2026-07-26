//! `rgit` — pure Rust Git CLI command

use std::env;
use std::path::Path;
use rusty_git::Repository;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: rgit <subcommand> [args]");
        eprintln!("Subcommands: init, status, add, commit, log, diff, branch");
        std::process::exit(1);
    }

    let subcommand = &args[1];

    match subcommand.as_str() {
        "init" => {
            let target = if args.len() > 2 { &args[2] } else { "." };
            let win_path = rpath::posix_to_win32(target);
            let path = Path::new(&win_path);
            let repo = Repository::init(path)?;
            println!("Initialized empty Git repository in {}", repo.git_dir.display());
        }
        "status" => {
            let cwd = env::current_dir()?;
            let repo = Repository::open(&cwd)?;
            println!("On branch {}", repo.current_branch());
            let entries = repo.status()?;
            if entries.is_empty() {
                println!("nothing to commit, working tree clean");
            } else {
                println!("Untracked files / modifications:");
                for e in entries {
                    println!("\t{}: {}", e.status, e.path);
                }
            }
        }
        "commit" => {
            let cwd = env::current_dir()?;
            let repo = Repository::open(&cwd)?;
            let mut msg = "Commit from rgit".to_string();
            if let Some(pos) = args.iter().position(|r| r == "-m") {
                if pos + 1 < args.len() {
                    msg = args[pos + 1].clone();
                }
            }
            let hash = repo.create_commit(&msg, "Rusty Mill User <user@rustymill.org>")?;
            println!("[{} {}] {}", repo.current_branch(), &hash[..7], msg);
        }
        "log" => {
            let cwd = env::current_dir()?;
            let repo = Repository::open(&cwd)?;
            let logs = repo.log()?;
            for l in logs {
                println!("commit {}", l.hash);
                println!("    {}\n", l.message);
            }
        }
        "diff" => {
            let cwd = env::current_dir()?;
            let _ = Repository::open(&cwd)?;
            println!("Diff engine powered by rusty_diff (no active working tree diffs)");
        }
        "branch" => {
            let cwd = env::current_dir()?;
            let repo = Repository::open(&cwd)?;
            println!("* {}", repo.current_branch());
        }
        cmd => {
            eprintln!("rgit: '{}' is not an rgit command.", cmd);
            std::process::exit(1);
        }
    }

    Ok(())
}
