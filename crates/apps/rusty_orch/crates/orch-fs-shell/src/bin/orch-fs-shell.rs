//! `orch-fs-shell --root <dir> --mode builder|critic`: serve one session's
//! root over MCP on stdin/stdout.

use std::process::ExitCode;
use std::sync::Arc;

use orch_fs_shell::{server, FsWorkspace, Role};

const USAGE: &str = "usage: orch-fs-shell --root <dir> --mode builder|critic";

/// What the command line asked for.
struct Args {
    root: String,
    role: Role,
}

fn parse(mut argv: impl Iterator<Item = String>) -> Result<Args, String> {
    let (mut root, mut role) = (None, None);
    while let Some(flag) = argv.next() {
        let value = argv
            .next()
            .ok_or_else(|| format!("`{flag}` needs a value"))?;
        match flag.as_str() {
            "--root" => root = Some(value),
            "--mode" => role = Some(mode(&value)?),
            _ => return Err(format!("unknown argument `{flag}`")),
        }
    }
    Ok(Args {
        root: root.ok_or("`--root` is required")?,
        role: role.ok_or("`--mode` is required")?,
    })
}

fn mode(value: &str) -> Result<Role, String> {
    match value {
        "builder" => Ok(Role::Builder),
        "critic" => Ok(Role::Critic),
        _ => Err(format!("`--mode` is builder or critic, not `{value}`")),
    }
}

fn run() -> Result<(), String> {
    let args = parse(std::env::args().skip(1))?;
    let ws = FsWorkspace::new(&args.root).map_err(|e| format!("--root {}: {e}", args.root))?;
    let server = server::build(ws, args.role).map_err(|e| e.to_string())?;
    rusty_mcp_server::serve_stdio(Arc::new(server)).map_err(|e| format!("stdio: {e}"))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("orch-fs-shell: {message}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn flags_parse_and_are_checked() {
        let ok = args(&["--mode", "critic", "--root", "/w"]).expect("parse");
        assert_eq!((ok.root.as_str(), ok.role), ("/w", Role::Critic));
        for (bad, want) in [
            (&["--root", "/w"][..], "--mode"),
            (&["--mode", "builder"][..], "--root"),
            (
                &["--mode", "admin", "--root", "/w"][..],
                "builder or critic",
            ),
            (&["--root"][..], "needs a value"),
            (&["--jail", "x"][..], "unknown"),
        ] {
            let err = args(bad).err().expect("refused");
            assert!(err.contains(want), "{bad:?}: {err}");
        }
    }
}
