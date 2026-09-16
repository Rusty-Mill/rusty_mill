//! `rpar cmd [args…] [-- cmd [args…]]…` — run commands concurrently and
//! report each as it finishes, in completion order. The reference
//! consumer for `wait_any`/`try_wait` (extraction map step 3): a
//! miniature parallel executor whose whole job is "block until whichever
//! child finishes next". Exits 0 only if every command did.

use platform::process::{Command, ExitStatus};

fn main() -> std::process::ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let commands: Vec<&[std::ffi::OsString]> = args
        .split(|a| a == "--")
        .filter(|c| !c.is_empty())
        .collect();
    if commands.is_empty() {
        eprintln!("usage: rpar cmd [args...] [-- cmd [args...]]...");
        return std::process::ExitCode::from(2);
    }
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("rpar: cannot determine cwd: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let spawner = coreutils::native::spawner();
    let mut children = Vec::new();
    let mut labels = Vec::new();
    let mut failed = false;
    for argv in &commands {
        let cmd = Command::new(&argv[0], cwd.clone()).args(argv[1..].iter().cloned());
        match spawner.spawn(&cmd) {
            Ok(child) => {
                labels.push(argv[0].to_string_lossy().into_owned());
                children.push(child);
            }
            Err(e) => {
                eprintln!("rpar: {e}");
                failed = true;
            }
        }
    }

    // Deferred signals (D6): the handler stores one atomic; this loop is
    // the safe point that consumes it — the reactor pattern of RFC §5.6
    // (children ∪ signals ∪ timeout) assembled from its pieces.
    use platform::events::SignalEvent;
    let signals = coreutils::native::signal_source();
    if let Err(e) = signals.install(&[SignalEvent::Interrupt, SignalEvent::Terminate]) {
        eprintln!("rpar: {e}");
    }

    // Completion-order reporting: the backend's multiplexed wait_any
    // (pidfd+poll / WaitForMultipleObjects) names whichever finished;
    // the consuming wait releases it. The wait ticks so a pending signal
    // is noticed within 200ms even while every child keeps running.
    while !children.is_empty() {
        if let Some(event) = signals.take() {
            eprintln!(
                "rpar: interrupted; stopping {} running command(s)",
                children.len()
            );
            for child in &children {
                let _ = child.kill_single(platform::process::Signal::Kill);
            }
            for child in children {
                let _ = child.wait();
            }
            return std::process::ExitCode::from(match event {
                SignalEvent::Interrupt => 130,
                _ => 143,
            });
        }
        let tick = std::time::Duration::from_millis(200);
        let idx = match spawner.wait_any(&mut children, Some(tick)) {
            Ok(Some(i)) => i,
            Ok(None) => continue, // tick elapsed — recheck signals
            Err(e) => {
                eprintln!("rpar: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        let label = labels.swap_remove(idx);
        let child = children.swap_remove(idx);
        match child.wait() {
            Ok(status) => {
                let ok = status.success();
                failed |= !ok;
                match status {
                    ExitStatus::Code(c) => println!("[{label}] exited {c}"),
                    ExitStatus::Signaled(s) => println!("[{label}] killed by signal {s}"),
                    ExitStatus::Stopped(_) | ExitStatus::Continued => unreachable!(
                        "Child::wait only ever produces Code/Signaled — Stopped/Continued are wait_job/try_wait_job-only (D10)"
                    ),
                }
            }
            Err(e) => {
                eprintln!("rpar: {label}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
