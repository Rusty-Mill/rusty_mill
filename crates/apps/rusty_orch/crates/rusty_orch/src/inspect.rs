//! Opt-in repository inspection at the I/O composition root. The bundled CI
//! helpers remain the single source of layer policy and reverse graph logic.

use std::path::PathBuf;
use std::time::Duration;

use orch_cli::{excerpt, CommandRunner};

use crate::args::ArgsError;

/// Inspection always emits a versioned JSON report. It never opens a task.
pub const USAGE: &str = "usage: rusty_orch inspect [--repo <dir>] [--impact <request.json>]\n\nRead-only repository intelligence or a revision-bound proposed-change report.\nRequires Cargo, Python 3.11+, a current Cargo.lock and cached dependencies.\nImpact requests use Millboard ownership and complete RemindMe snapshots.\nNo agents, builds, tests or changes run.";

/// Options for the read-only interface; independent of goal execution.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    /// Checkout to inspect.
    pub repo: PathBuf,
    /// Proposed change and explicit snapshot locations.
    pub impact: Option<PathBuf>,
}

/// Parse the extended interface without changing the original repository parser.
pub fn options(argv: impl IntoIterator<Item = String>, cwd: PathBuf) -> Result<Options, ArgsError> {
    let mut args = argv.into_iter();
    let mut remaining = Vec::new();
    let mut impact = None;
    while let Some(arg) = args.next() {
        if arg != "--impact" {
            remaining.push(arg);
            continue;
        }
        if impact.is_some() {
            return Err(ArgsError::Usage("duplicate --impact".into()));
        }
        impact = Some(PathBuf::from(
            args.next()
                .filter(|v| !v.is_empty() && !v.starts_with('-'))
                .ok_or_else(|| ArgsError::Usage("--impact needs a request file".into()))?,
        ));
    }
    Ok(Options {
        repo: parse(remaining, cwd)?,
        impact,
    })
}

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
        ("repository_intelligence", include_str!("intelligence.py")),
    ];
    let mut program = String::from("import sys, types\nif sys.version_info < (3, 11): sys.exit(\"repository inspection requires Python 3.11+\")\n");
    for (name, source) in modules {
        let literal = rusty_serde::json::to_string(source)?;
        program.push_str(&format!(
            "m = types.ModuleType('{name}')\nsys.modules['{name}'] = m\nexec({literal}, m.__dict__)\n"
        ));
    }
    program.push_str(include_str!("impact.py"));
    Ok(program)
}

/// Discover and analyze with the existing bounded process seam. Failure returns
/// no partial report; findings themselves are a successful inspection result.
pub fn inspect(
    repo: &std::path::Path,
    runner: &impl CommandRunner,
) -> Result<String, Box<dyn std::error::Error>> {
    inspect_with_impact(repo, None, runner)
}

/// Like [`inspect`], optionally joining complete, explicitly supplied snapshots.
/// The same isolated process, output limit and deadline apply to both modes.
pub fn inspect_with_impact(
    repo: &std::path::Path,
    request: Option<&std::path::Path>,
    runner: &impl CommandRunner,
) -> Result<String, Box<dyn std::error::Error>> {
    Ok(inspect_with_impact_diagnostics(repo, request, runner)?.report)
}

/// Successful inspection output with explicitly retained helper diagnostics.
pub struct InspectionOutput {
    /// Stable JSON report written to stdout by the normal interface.
    pub report: String,
    /// Opt-in diagnostics emitted by the helper on a successful invocation.
    pub diagnostics: String,
}

/// Run inspection while explicitly retaining stderr from a successful helper.
pub fn inspect_with_impact_diagnostics(
    repo: &std::path::Path,
    request: Option<&std::path::Path>,
    runner: &impl CommandRunner,
) -> Result<InspectionOutput, Box<dyn std::error::Error>> {
    let repo = repo.canonicalize()?;
    let mut argv = vec![
        "python3".to_owned(),
        "-I".to_owned(),
        "-".to_owned(),
        repo.to_str()
            .ok_or("repository path must be UTF-8")?
            .to_owned(),
    ];
    if let Some(request) = request {
        argv.push(
            request
                .canonicalize()?
                .to_str()
                .ok_or("request path must be UTF-8")?
                .to_owned(),
        );
    }
    // The compiled-in source exceeds Windows' CreateProcess command-line
    // limit. Python's stdin script mode keeps argv bounded and, with -I,
    // still excludes target-repository imports. No script is written to disk.
    let program = bundled_program()?;
    let exit = runner.run_in(
        Some(&repo),
        &argv,
        program.as_bytes(),
        Duration::from_secs(120),
        &[],
    )?;
    if exit.status != 0 {
        return Err(format!("repository inspection: {}", excerpt(&exit.stderr)).into());
    }
    let report: rusty_json::Value = rusty_json::Value::parse(std::str::from_utf8(&exit.stdout)?)?;
    Ok(InspectionOutput {
        report: report.to_json_string_pretty(),
        diagnostics: String::from_utf8_lossy(&exit.stderr).into_owned(),
    })
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

    #[test]
    fn successful_subprocess_diagnostics_require_explicit_opt_in() {
        let repo = std::env::current_dir().expect("cwd");
        let runner = FakeCommand::new(Ok(exit(0, "{}", "diagnostic")));
        assert_eq!(inspect(&repo, &runner).expect("report"), "{}");
        let output = inspect_with_impact_diagnostics(&repo, None, &runner).expect("output");
        assert_eq!(output.report, "{}");
        assert_eq!(output.diagnostics, "diagnostic");
    }

    #[test]
    fn report_only_inspection_accepts_non_utf8_success_stderr() {
        let repo = std::env::current_dir().expect("cwd");
        let mut success = exit(0, "{}", "");
        success.stderr = vec![0xff];
        let runner = FakeCommand::new(Ok(success));
        assert_eq!(inspect(&repo, &runner).expect("report"), "{}");
    }

    #[test]
    fn bundled_source_uses_stdin_with_isolation_and_bounded_launch_arguments() {
        let repo = std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .expect("repo");
        let request = repo
            .join("Cargo.toml")
            .canonicalize()
            .expect("existing file");
        for impact in [None, Some(request.as_path())] {
            let runner = FakeCommand::ok("{}");
            inspect_with_impact(&repo, impact, &runner).expect("report");
            let calls = runner.calls.lock().expect("calls");
            let (argv, stdin, timeout) = &calls[0];
            assert_eq!(&argv[..3], ["python3", "-I", "-"]);
            assert_eq!(argv.len(), if impact.is_some() { 5 } else { 4 });
            assert_eq!(argv[3], repo.to_str().expect("UTF8"));
            if impact.is_some() {
                assert_eq!(argv[4], request.to_str().expect("UTF8"));
            }
            assert_eq!(stdin, bundled_program().expect("source").as_bytes());
            assert!(argv.iter().map(|s| s.encode_utf16().count()).sum::<usize>() < 1024);
            assert_eq!(*timeout, Duration::from_secs(120));
            assert_eq!(
                runner.cwds.lock().expect("cwds")[0].as_deref(),
                Some(repo.as_path())
            );
        }
    }

    #[test]
    fn impact_options_keep_execution_flags_out_and_reject_missing_or_duplicate_files() {
        let parsed = options(
            ["--impact", "request.json", "--repo", "checkout"].map(str::to_owned),
            PathBuf::from("."),
        )
        .expect("options");
        assert_eq!(parsed.repo, PathBuf::from("checkout"));
        assert_eq!(parsed.impact, Some(PathBuf::from("request.json")));
        for args in [
            vec!["--impact"],
            vec!["--impact", "--repo"],
            vec!["--impact", "a", "--impact", "b"],
            vec!["--impact", "a", "--state", "x"],
        ] {
            assert!(matches!(
                options(args.into_iter().map(str::to_owned), PathBuf::from(".")),
                Err(ArgsError::Usage(_))
            ));
        }
    }
}
