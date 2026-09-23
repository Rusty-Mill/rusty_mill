//! Refresh a cold-standby replica from a running server (`ADR-0118`):
//! `Request::FetchSnapshot` (`ADR-0067`) written into a fresh, verified
//! directory, once or on an interval — the operator recipe
//! `docs/design/SERVER-REPLICATION-DESIGN.md` named, as running code.
//!
//! Run: `REPLICA_REFRESH_TOKEN=<replication token> cargo run -p
//! rusty_multimodal_db --features client --example replica_refresh --
//! <host:port> <root_dir> <memory|entity|relation> [--every <secs>] [--keep <n>]`.
//!
//! Each refresh makes `<root_dir>/<secs>-<pid>-<seq>/` holding the
//! table's files, crash-safely (staged, synced, renamed in one step,
//! verified by a real reopen). To use one: stop the standby server,
//! point its `SERVER_DATA_DIR` at that directory, start it. `--every`
//! repeats until interrupted; `--keep` removes all but the newest `n`
//! after each refresh. The token comes from the environment, never
//! the command line, so it is not in the process list. Plaintext
//! transport: run it on the server's host or a trusted network, or
//! front the server with TLS and add `ConnectOptions::tls` here.
#[path = "support/replica_refresh_lib.rs"]
mod replica_refresh;

use replica_refresh::{prune, refresh, Domain, Target};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

fn usage() -> ExitCode {
    eprintln!(
        "usage: REPLICA_REFRESH_TOKEN=<token> replica_refresh <host:port> <root_dir> \
         <memory|entity|relation> [--every <secs>] [--keep <n>]"
    );
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return usage();
    }
    let addr = args[0].clone();
    let root = PathBuf::from(&args[1]);
    let domain = match args[2].parse::<Domain>() {
        Ok(domain) => domain,
        Err(error) => {
            eprintln!("{error}");
            return usage();
        }
    };
    let mut every: Option<u64> = None;
    let mut keep: usize = 0;
    let mut rest = args[3..].iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().and_then(|v| v.parse::<u64>().ok());
        match (flag.as_str(), value) {
            ("--every", Some(secs)) if secs > 0 => every = Some(secs),
            ("--keep", Some(n)) => keep = n as usize,
            _ => return usage(),
        }
    }
    let Some(token) = std::env::var_os("REPLICA_REFRESH_TOKEN") else {
        eprintln!(
            "REPLICA_REFRESH_TOKEN is not set: the replication token comes from the environment"
        );
        return usage();
    };
    let target = Target::new(addr, &token.to_string_lossy());

    loop {
        match refresh(&target, &root, domain) {
            Ok(report) => {
                println!(
                    "refreshed {} file(s), {} byte(s), {} record(s) into {}",
                    report.files,
                    report.bytes,
                    report.records,
                    report.directory.display()
                );
                match prune(&root, keep) {
                    Ok(removed) => {
                        for dir in removed {
                            println!("removed {}", dir.display());
                        }
                    }
                    Err(error) => eprintln!("pruning {}: {error}", root.display()),
                }
                if every.is_none() {
                    println!(
                        "Stop the standby server, set SERVER_DATA_DIR to {} and start it.",
                        report.directory.display()
                    );
                    return ExitCode::SUCCESS;
                }
            }
            Err(error) => {
                eprintln!("refresh failed: {error}");
                if every.is_none() {
                    return ExitCode::FAILURE;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(every.unwrap_or(0)));
    }
}
