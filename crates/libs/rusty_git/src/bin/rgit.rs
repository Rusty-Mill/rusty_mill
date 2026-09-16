//! `rgit` — a real (if intentionally scoped-down) pure-Rust Git CLI.
//! See `rusty_git`'s own crate-level doc for what's genuinely implemented
//! vs. known, documented gaps.

use std::env;
use std::path::Path;

use rusty_git::objects::{read_object, ObjectKind};
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
            println!(
                "Initialized empty Git repository in {}",
                repo.git_dir.display()
            );
        }
        "add" => {
            let cwd = env::current_dir()?;
            let repo = Repository::open(&cwd)?;
            let paths: Vec<String> = args[2..].to_vec();
            if paths.is_empty() {
                eprintln!("Nothing specified, nothing added.\nUsage: rgit add <path>...");
                std::process::exit(1);
            }
            repo.add(&paths)?;
        }
        "status" => {
            let cwd = env::current_dir()?;
            let repo = Repository::open(&cwd)?;
            println!("On branch {}", repo.current_branch());
            let entries = repo.status()?;
            if entries.is_empty() {
                println!("nothing to commit, working tree clean");
            } else {
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
                println!("Author: {}", l.author);
                println!("\n    {}\n", l.message);
            }
        }
        "diff" => {
            let cwd = env::current_dir()?;
            let repo = Repository::open(&cwd)?;
            let index = repo.index()?;
            let entries = repo.status()?;
            let mut printed_any = false;
            for e in entries {
                if e.status != "modified (not staged)" {
                    continue;
                }
                let Some(idx_entry) = index.get(&e.path) else {
                    continue;
                };
                let old_oid = rusty_git::sha1::hex(&idx_entry.hash);
                let (kind, old_content) = read_object(&repo.git_dir, &old_oid)?;
                if kind != ObjectKind::Blob {
                    continue;
                }
                let new_content = std::fs::read(repo.work_tree.join(&e.path))?;
                let old_text = String::from_utf8_lossy(&old_content);
                let new_text = String::from_utf8_lossy(&new_content);
                print!(
                    "{}",
                    rusty_diff::format_unified_diff(&e.path, &e.path, &old_text, &new_text)
                );
                printed_any = true;
            }
            if !printed_any {
                println!("(no unstaged modifications to a tracked file)");
            }
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
