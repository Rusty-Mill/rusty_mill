//! Reference tool 2: spawns a child process through a contract
//! `ProcessRunner`, captures stdout/stderr, and reports the exit status.
//! Exercises process spawn + stdio capture only — no fs, no PTY.

use anyhow::bail;
use compat::NativeProcessRunner;
use contract::{ProcessRunner, ProcessSpec};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(program) = args.next() else {
        bail!("usage: proc-runner <program> [args...]");
    };

    let mut spec = ProcessSpec::new(program);
    for arg in args {
        spec = spec.arg(arg);
    }

    let runner = NativeProcessRunner;
    let output = runner.run(&spec)?;

    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    std::process::exit(output.status);
}
