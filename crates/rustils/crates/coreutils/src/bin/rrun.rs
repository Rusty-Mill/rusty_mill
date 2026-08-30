//! `rrun <program> [args…]` — spawn a program through the platform
//! `Spawner` and propagate its exit status. The reference consumer that
//! gates the process domain's native backends (RFC v2 §3): resolve, the
//! full spawn path (winargv on Windows), consuming wait, and decoded
//! `ExitStatus` all get exercised end to end by one binary.

use platform::process::{Command, ExitStatus};

fn main() -> std::process::ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(program) = args.next() else {
        eprintln!("usage: rrun <program> [args...]");
        return std::process::ExitCode::from(2);
    };
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("rrun: cannot determine cwd: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let cmd = Command::new(program, cwd).args(args);
    let spawner = coreutils::native::spawner();
    let child = match spawner.spawn(&cmd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rrun: {e}");
            return std::process::ExitCode::from(127);
        }
    };
    match child.wait() {
        // The shell convention for signal deaths (128+n); the decoded
        // ExitStatus is what makes this expressible portably.
        Ok(ExitStatus::Code(code)) => std::process::ExitCode::from((code & 0xff) as u8),
        Ok(ExitStatus::Signaled(sig)) => std::process::ExitCode::from((128 + (sig & 0x7f)) as u8),
        // Child::wait only ever produces Code/Signaled — Stopped/Continued
        // are wait_job/try_wait_job-only (D10).
        Ok(ExitStatus::Stopped(_) | ExitStatus::Continued) => unreachable!(),
        Err(e) => {
            eprintln!("rrun: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
