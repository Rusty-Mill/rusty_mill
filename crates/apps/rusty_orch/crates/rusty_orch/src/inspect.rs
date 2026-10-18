//! Opt-in repository inspection at the I/O composition root. The bundled CI
//! helpers remain the single source of layer policy and reverse graph logic.

use std::path::PathBuf;
use std::time::Duration;

use orch_cli::{excerpt, CommandRunner};

use crate::args::ArgsError;

/// Inspection always emits a versioned JSON report. It never opens a task.
pub const USAGE: &str = "usage: rusty_orch inspect [--repo <dir>]\n\nRead-only repository intelligence as JSON. Requires Cargo, Python 3.11+, a current\nCargo.lock and cached dependencies. No agents, builds, tests or changes run.";

/// Parse inspection options separately from goal execution and its environment.
pub fn parse(argv: impl IntoIterator<Item = String>, cwd: PathBuf) -> Result<PathBuf, ArgsError> {
    let mut argv = argv.into_iter();
    let mut repo = None;
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--repo" if repo.is_none() => {
                repo = Some(PathBuf::from(
                    argv.next()
                        .filter(|s| !s.is_empty() && !s.starts_with('-'))
                        .ok_or_else(|| ArgsError::Usage("--repo needs a directory".into()))?,
                ));
            }
            "-h" | "--help" => return Err(ArgsError::Help),
            _ => {
                return Err(ArgsError::Usage(format!(
                    "unexpected inspection argument {arg:?}"
                )))
            }
        }
    }
    Ok(repo.unwrap_or(cwd))
}

fn bundled_program() -> Result<String, Box<dyn std::error::Error>> {
    // Compile-time source inclusion: never execute policy scripts or Python
    // imports supplied by the target repository. -I also excludes cwd/PYTHONPATH.
    let modules = [
        (
            "affected_crates",
            include_str!("../../../../../../.github/scripts/affected_crates.py"),
        ),
        (
            "check_workspace_layers",
            include_str!("../../../../../../.github/scripts/check_workspace_layers.py"),
        ),
        (
            "check_workspace_deps",
            include_str!("../../../../../../.github/scripts/check_workspace_deps.py"),
        ),
    ];
    let mut program = String::from("import sys, types\nif sys.version_info < (3, 11): sys.exit(\"repository inspection requires Python 3.11+\")\n");
    for (name, source) in modules {
        let literal = rusty_serde::json::to_string(source)?;
        program.push_str(&format!(
            "m = types.ModuleType('{name}')\nsys.modules['{name}'] = m\nexec({literal}, m.__dict__)\n"
        ));
    }
    program.push_str(include_str!("intelligence.py"));
    Ok(program)
}

/// Discover and analyze with the existing bounded process seam. Failure returns
/// no partial report; findings themselves are a successful inspection result.
pub fn inspect(
    repo: &std::path::Path,
    runner: &impl CommandRunner,
) -> Result<String, Box<dyn std::error::Error>> {
    let repo = repo.canonicalize()?;
    let argv = vec![
        "python3".to_owned(),
        "-I".to_owned(),
        "-c".to_owned(),
        bundled_program()?,
        repo.to_str()
            .ok_or("repository path must be UTF-8")?
            .to_owned(),
    ];
    let exit = runner.run_in(Some(&repo), &argv, &[], Duration::from_secs(120), &[])?;
    if exit.status != 0 {
        return Err(format!("repository inspection: {}", excerpt(&exit.stderr)).into());
    }
    let report: rusty_json::Value = rusty_json::Value::parse(std::str::from_utf8(&exit.stdout)?)?;
    Ok(report.to_json_string_pretty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_cli::fake::{exit, FakeCommand};

    #[test]
    fn inspection_options_are_isolated_from_run() {
        assert_eq!(parse([], PathBuf::from(".")), Ok(PathBuf::from(".")));
        for options in [
            vec!["--state", "state"],
            vec!["--repo"],
            vec!["--repo", "--help"],
            vec!["--repo", "a", "--repo", "b"],
            vec!["--bbp", "config"],
        ] {
            assert!(matches!(
                parse(options.into_iter().map(str::to_owned), PathBuf::from(".")),
                Err(ArgsError::Usage(_))
            ));
        }
    }

    #[test]
    fn subprocess_errors_do_not_become_clean_reports() {
        let runner = FakeCommand::new(Ok(exit(1, "", "cargo metadata failed")));
        let error = inspect(&std::env::current_dir().expect("cwd"), &runner).expect_err("failure");
        assert!(error.to_string().contains("cargo metadata failed"));
    }
}
