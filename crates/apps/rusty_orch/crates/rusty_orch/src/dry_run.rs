//! `rusty_orch dry-run` (#598, Let Em Loose #758 item 3, D138): a dry run
//! over the BBP config's `real_data` roots, through `bbp dry-run`.
//!
//! The program runs confined by the host with no network, reading the
//! named roots and writing only its plan directory. The plan is sensitive
//! and stays on the host: `bbp dry-run` prints only its summary (plan path,
//! digest and line count), and this prints that and nothing else.

use std::path::PathBuf;
use std::process::Command;

use crate::args::ArgsError;

pub const USAGE: &str = "usage: rusty_orch dry-run --bbp <bbp config.json> --name <name> --work <dir> -- <program> [args...]\n\nRun <program> as a BBP dry run over every `real_data` root the BBP config names\n(`bbp dry-run`): confined, offline, reading those roots and writing only its plan\nunder <dir>. Prints BBP's summary of the plan, never the plan itself.";

/// What to run.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub bbp: PathBuf,
    pub name: String,
    pub work: String,
    pub program: String,
    pub args: Vec<String>,
}

/// Parse `dry-run --bbp … --name … --work … -- PROGRAM ARGS…`.
pub fn options(argv: impl IntoIterator<Item = String>) -> Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    if args.next().as_deref() != Some("dry-run") {
        return Err(ArgsError::Help);
    }
    let (mut bbp, mut name, mut work) = (None, None, None);
    let mut program = None;
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "-h" | "--help" => return Err(ArgsError::Help),
            "--" => {
                program = args.next();
                break;
            }
            "--bbp" => &mut bbp,
            "--name" => &mut name,
            "--work" => &mut work,
            _ => return Err(ArgsError::Usage(format!("unexpected argument {arg:?}"))),
        };
        if slot.is_some() {
            return Err(ArgsError::Usage(format!("duplicate {arg}")));
        }
        let value = args
            .next()
            .filter(|v| !v.is_empty() && !v.starts_with('-'))
            .ok_or_else(|| ArgsError::Usage(format!("{arg} needs a value")))?;
        *slot = Some(value);
    }
    let required =
        |v: Option<String>, f: &str| v.ok_or_else(|| ArgsError::Usage(format!("{f} is required")));
    Ok(Options {
        bbp: required(bbp, "--bbp")?.into(),
        name: required(name, "--name")?,
        work: required(work, "--work")?,
        program: required(program, "a program after --")?,
        args: args.collect(),
    })
}

/// Read and check the BBP config, then run `bbp dry-run` and return its
/// summary line.
pub fn run(o: &Options) -> Result<String, String> {
    let text = std::fs::read_to_string(&o.bbp)
        .map_err(|e| format!("cannot read {}: {e}", o.bbp.display()))?;
    let config: orch_bbp::Config =
        rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", o.bbp.display()))?;
    let config = config.absolute()?;
    config.validate()?;
    let argv = orch_bbp::dry_run_args(&config, &o.name, &o.work, &o.program, &o.args)?;
    let out = Command::new(&config.binary)
        .args(&argv)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", config.binary.display()))?;
    if !out.status.success() {
        return Err(format!(
            "bbp dry-run: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let summary = String::from_utf8(out.stdout).map_err(|e| format!("bbp dry-run: {e}"))?;
    Ok(format!("{}\n", summary.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn everything_after_the_separator_is_the_program() {
        let o = options(argv(
            "dry-run --bbp c.json --name n --work /w -- prog --all -x",
        ))
        .expect("parses");
        assert_eq!(o.program, "prog");
        assert_eq!(o.args, ["--all", "-x"]);
        assert_eq!((o.name.as_str(), o.work.as_str()), ("n", "/w"));
    }

    #[test]
    fn missing_or_repeated_flags_are_refused() {
        let err = |s: &str| match options(argv(s)) {
            Err(ArgsError::Usage(m)) => m,
            other => panic!("{other:?}"),
        };
        assert!(err("dry-run --bbp c --name n --work /w").contains("program"));
        assert!(err("dry-run --bbp c --work /w -- p").contains("--name"));
        assert!(err("dry-run --name a --name b").contains("duplicate"));
        assert!(err("dry-run p").contains("unexpected"));
    }
}
