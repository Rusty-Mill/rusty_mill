#![cfg(target_os = "linux")]
//! `arun <program> [args…]` — async port of rustils' `rrun`: spawn a
//! program through [`platform_async::process::AsyncSpawner`] and
//! propagate its exit status, awaiting termination through
//! `platform-async-linux`'s pidfd + epoll reactor instead of a blocking
//! wait. Plays the same forcing-consumer role for the async process
//! surface that `rrun` plays for the sync one — see that binary's own
//! doc comment in rustils: "the reference consumer that gates the
//! process domain's native backends."

use platform::process::{Command, ExitStatus};
use platform_async::process::AsyncSpawner;
use platform_async_linux::AsyncLinuxSpawner;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(program) = args.next() else {
        eprintln!("usage: arun <program> [args...]");
        return std::process::ExitCode::from(2);
    };
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("arun: cannot determine cwd: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let cmd = Command::new(program, cwd).args(args);

    let spawner = match AsyncLinuxSpawner::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("arun: cannot start reactor: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let child = match spawner.spawn(&cmd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("arun: {e}");
            return std::process::ExitCode::from(127);
        }
    };
    match coreutils_async::block_on(child.wait()) {
        // The shell convention for signal deaths (128+n) — same as
        // `rrun`; the decoded `ExitStatus` is what makes this
        // expressible portably.
        Ok(ExitStatus::Code(code)) => std::process::ExitCode::from((code & 0xff) as u8),
        Ok(ExitStatus::Signaled(sig)) => std::process::ExitCode::from((128 + (sig & 0x7f)) as u8),
        // AsyncChild::wait only ever produces Code/Signaled, same as
        // the sync Child::wait it wraps (Stopped/Continued are
        // wait_job/try_wait_job-only, D10 — no async counterpart here).
        Ok(ExitStatus::Stopped(_) | ExitStatus::Continued) => unreachable!(),
        Err(e) => {
            eprintln!("arun: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
