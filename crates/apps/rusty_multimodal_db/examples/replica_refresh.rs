//! Refresh a cold-standby replica from a running server (`ADR-0118`):
//! `Request::FetchSnapshot` (`ADR-0067`) written into a fresh, verified
//! directory, once or on an interval — the operator recipe
//! `docs/design/SERVER-REPLICATION-DESIGN.md` named, as running code.
//!
//! Run: `REPLICA_REFRESH_TOKEN=<replication token> cargo run -p
//! rusty_multimodal_db --features client --example replica_refresh --
//! <host:port> <root_dir> <memory|entity|relation> [--every <secs>] [--keep <n>] [--follow]`.
//!
//! Each refresh makes `<root_dir>/<secs>-<pid>-<seq>/` holding the
//! table's files, crash-safely (staged, synced, renamed in one step,
//! verified by a real reopen). To use one: stop the standby server,
//! point its `SERVER_DATA_DIR` at that directory, start it. `--every`
//! repeats until interrupted; `--keep` removes all but the newest `n`
//! after each refresh. The token comes from the environment, never
//! the command line, so it is not in the process list. Set
//! `REPLICA_REFRESH_TLS_SERVER_NAME=<name>` to connect over TLS to a
//! server presenting a certificate for `<name>`, verified against the
//! operating system's trust anchors (`SSL_CERT_FILE` names a private
//! CA's bundle); unset, the transport is plaintext, for the server's
//! own host or a trusted network only (`ADR-0123`).
//!
//! `--follow` (`ADR-0131`, for a server with `SERVER_CHANGE_LOG_DIR`)
//! refreshes once, then keeps tailing the primary's change log into that
//! directory, one batch at a time, recording its position; if the log no
//! longer reaches it the tool refreshes again and carries on. **Promotion**
//! is manual: stop the tool, start a server with `SERVER_DATA_DIR` at the
//! directory — it is a complete table. No failover, no write forwarding.
#[path = "support/replica_refresh_lib.rs"]
mod replica_refresh;

#[cfg(feature = "server")]
use replica_refresh::{follow, refresh_at, FollowError};
use replica_refresh::{prune, refresh, refresh_loop, Domain, RefreshError, RefreshReport, Target};
#[cfg(feature = "server")]
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

fn usage() -> ExitCode {
    eprintln!(
        "usage: REPLICA_REFRESH_TOKEN=<token> replica_refresh <host:port> <root_dir> \
         <memory|entity|relation> [--every <secs>] [--keep <n>] [--follow]"
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
    let mut follow_log = false;
    let mut rest = args[3..].iter().peekable();
    while let Some(flag) = rest.next() {
        if flag == "--follow" {
            follow_log = true;
            continue;
        }
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
    let token = token.to_string_lossy();
    let target = match std::env::var("REPLICA_REFRESH_TLS_SERVER_NAME") {
        Ok(name) if !name.is_empty() => Target::with_tls(addr, &token, &name),
        _ => Target::new(addr, &token),
    };

    let print = |outcome: &Result<RefreshReport, RefreshError>,
                 pruned: &std::io::Result<Vec<PathBuf>>| {
        match outcome {
            Ok(report) => println!(
                "refreshed {} file(s), {} byte(s), {} record(s) into {}",
                report.files,
                report.bytes,
                report.records,
                report.directory.display()
            ),
            Err(error) => eprintln!("refresh failed: {error}"),
        }
        match pruned {
            Ok(removed) => {
                for dir in removed {
                    println!("removed {}", dir.display());
                }
            }
            Err(error) => eprintln!("pruning {}: {error}", root.display()),
        }
    };

    if follow_log {
        #[cfg(feature = "server")]
        return follow_forever(&target, &root, domain, keep);
        #[cfg(not(feature = "server"))]
        {
            eprintln!("--follow needs this example built with the `server` feature");
            return ExitCode::FAILURE;
        }
    }
    let Some(secs) = every else {
        let outcome = refresh(&target, &root, domain);
        let pruned = if outcome.is_ok() {
            prune(&root, keep)
        } else {
            Ok(Vec::new())
        };
        print(&outcome, &pruned);
        return match outcome {
            Ok(report) => {
                println!(
                    "Stop the standby server, set SERVER_DATA_DIR to {} and start it.",
                    report.directory.display()
                );
                ExitCode::SUCCESS
            }
            Err(_) => ExitCode::FAILURE,
        };
    };
    refresh_loop(
        &target,
        &root,
        domain,
        keep,
        Duration::from_secs(secs),
        |outcome, pruned| {
            print(outcome, pruned);
            true
        },
    );
    ExitCode::SUCCESS
}

#[cfg(feature = "server")]
/// `--follow`: refresh with a position, tail it, and start over when the
/// log no longer reaches this standby.
fn follow_forever(target: &Target, root: &Path, domain: Domain, keep: usize) -> ExitCode {
    loop {
        let report = match refresh_at(target, root, domain) {
            Ok(report) => report,
            Err(error) => {
                eprintln!("refresh failed: {error}");
                return ExitCode::FAILURE;
            }
        };
        println!(
            "snapshot in {} at {:?}; following",
            report.directory.display(),
            report.position
        );
        let _ = prune(root, keep);
        let outcome = follow(
            target,
            &report.directory,
            domain,
            Duration::from_secs(1),
            |applied, head| {
                println!("applied through {applied} (primary head {head})");
                true
            },
        );
        match outcome {
            Err(FollowError::Resync) => eprintln!("{}", FollowError::Resync),
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
            Ok(()) => return ExitCode::SUCCESS,
        }
    }
}
