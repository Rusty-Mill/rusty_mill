//! `lel-jail JAIL.json`: the program a mux agent block runs (LEL D104).
//!
//! Exits 125 (rusty_sandbox's `SETUP_FAILED`) with the reason on stderr,
//! which is the pty, when the jail can't be set up; otherwise it becomes
//! the agent.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    let [spec] = args.as_slice() else {
        eprintln!("usage: lel-jail JAIL.json");
        return ExitCode::from(2);
    };
    eprintln!("lel-jail: {}", run(spec));
    ExitCode::from(rusty_sandbox::SETUP_FAILED)
}

#[cfg(target_os = "linux")]
fn run(spec: &std::path::Path) -> String {
    orch_jail::launch::launch(spec)
}

#[cfg(not(target_os = "linux"))]
fn run(_spec: &std::path::Path) -> String {
    "the agent jail is only implemented on Linux".into()
}
