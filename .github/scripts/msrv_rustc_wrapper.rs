//! `RUSTC_WRAPPER` for `check_msrv_compile.py`: run the real rustc (the first argument)
//! without what a newer cargo adds that an older rustc rejects and that cannot change
//! whether the code compiles: `--check-cfg` (unstable before rustc 1.80) and the
//! `--warn=`/`--allow=`/`--deny=`/`--forbid=` flags cargo derives from a dependency's
//! `[lints]` table (an unknown lint name on the command line is a hard error in old rustc).
//!
//! Rust rather than a shell script so it runs on Windows too. Cargo moves a long command
//! line into an `@argfile` (one argument per line), so those are filtered the same way.

use std::ffi::OsString;
use std::process::{exit, Command};

const DROPPED_PREFIXES: [&str; 5] = ["--check-cfg=", "--warn=", "--allow=", "--deny=", "--forbid="];

/// Drop the flags old rustc rejects; `--check-cfg` takes its value as the next argument.
fn strip(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut kept = Vec::new();
    let mut skip_value = false;
    for arg in args {
        if std::mem::take(&mut skip_value) {
            continue;
        }
        let text = arg.to_string_lossy();
        if text == "--check-cfg" {
            skip_value = true;
        } else if !DROPPED_PREFIXES.iter().any(|p| text.starts_with(p)) {
            kept.push(arg);
        }
    }
    kept
}

/// Rewrite an `@argfile` argument to a filtered copy; any other argument passes through.
fn filter_argfile(arg: OsString) -> std::io::Result<OsString> {
    let Some(path) = arg.to_str().and_then(|a| a.strip_prefix('@')) else {
        return Ok(arg);
    };
    let lines = std::fs::read_to_string(path)?.lines().map(OsString::from).collect::<Vec<_>>();
    let kept = strip(lines).iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
    let filtered = format!("{path}.msrv");
    std::fs::write(&filtered, kept.join("\n"))?;
    Ok(OsString::from(format!("@{filtered}")))
}

fn run() -> std::io::Result<i32> {
    let mut args = std::env::args_os().skip(1);
    let rustc = args
        .next()
        .ok_or_else(|| std::io::Error::other("usage: msrv_rustc_wrapper RUSTC [ARGS...]"))?;
    let args = strip(args).into_iter().map(filter_argfile).collect::<std::io::Result<Vec<_>>>()?;
    Ok(Command::new(rustc).args(args).status()?.code().unwrap_or(1))
}

fn main() {
    match run() {
        Ok(code) => exit(code),
        Err(e) => {
            eprintln!("msrv_rustc_wrapper: {e}");
            exit(1);
        }
    }
}
