//! Spawn target for the process probes in `conformance`.
//!
//! Deliberately trivial and host-neutral: the process probes must compare
//! the *contract's* spawn behavior across hosts, not the availability of
//! `echo` vs `cmd /C echo`. Anything that differs per host here would be
//! measuring the host's shell, not `ProcessRunner`.

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    match mode.as_str() {
        // Writes a known marker to each stream and exits non-zero, so a
        // probe can assert stdout, stderr, and exit status independently.
        "streams" => {
            print!("STDOUT-MARKER");
            eprint!("STDERR-MARKER");
            std::process::exit(7);
        }
        // Prints the child's view of one environment variable, so a probe
        // can prove `inherit_env: false` really starts from an empty env.
        "getenv" => {
            let key = std::env::args().nth(2).unwrap_or_default();
            match std::env::var(&key) {
                Ok(value) => print!("SET={value}"),
                Err(_) => print!("UNSET"),
            }
        }
        // PTY payload: echoes back the marker it is given, then exits with a
        // known code. Reads nothing from stdin and loads no rc files, so a
        // PTY probe measures the contract rather than the user's shell.
        "pty" => {
            let marker = std::env::args().nth(2).unwrap_or_default();
            print!("{marker}");
            // A PTY delivers bytes as the child writes them; without an
            // explicit flush a short marker can sit in the child's buffer
            // until exit, which would make the probe time-dependent.
            use std::io::Write;
            std::io::stdout().flush().expect("flush pty marker");
            std::process::exit(11);
        }
        // Prints the child's working directory, for the cwd guarantee.
        "cwd" => {
            let cwd = std::env::current_dir().expect("child cwd");
            print!("{}", cwd.display());
        }
        other => {
            eprint!("unknown child mode: {other}");
            std::process::exit(2);
        }
    }
}
