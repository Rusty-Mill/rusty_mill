//! `rsi inner`: one inner run of a harness on one task, with a live model.
//!
//! ```text
//! rsi inner --harness DIR --task DIR --tokens N --wall-secs N
//!           [--seed N] [--work DIR] [--transcript FILE]
//! ```
//!
//! The inner model comes from the environment (`RSI_INNER_*`, see
//! [`crate::config`]), never from flags or files that might be logged.
//! `RSI_RUSTC` names the compiler that builds the harness (default
//! `rustc`).

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::inner_from_env;
use rsi_core::{Budget, Harness, PublicTask, Seed};
use rsi_runtime::grading::SYSTEM_READ_ROOTS;
use rsi_runtime::harness::build;
use rsi_runtime::{
    HarnessProcess, LocalTask, ProcessExecutor, RuntimeError, SandboxedHarness, SolutionRunner,
    TaskDir, Toolchain,
};

/// Completion tokens asked for per model call.
const MAX_COMPLETION_TOKENS: u64 = 4096;
/// How long past its wall-clock budget the agent may run.
const GRACE: Duration = Duration::from_secs(5);

/// Parsed `rsi inner` arguments.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    harness: PathBuf,
    task: PathBuf,
    tokens: u64,
    wall: Duration,
    seed: u64,
    work: Option<PathBuf>,
    transcript: Option<PathBuf>,
}

fn parse(args: &[OsString]) -> Result<Args, String> {
    let (mut harness, mut task, mut tokens, mut wall) = (None, None, None, None);
    let (mut seed, mut work, mut transcript) = (0, None, None);
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let flag = flag.to_string_lossy();
        let value = iter.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let number = || {
            value
                .to_str()
                .and_then(|v| v.parse::<u64>().ok())
                .ok_or_else(|| format!("{flag} needs a number"))
        };
        match flag.as_ref() {
            "--harness" => harness = Some(PathBuf::from(value)),
            "--task" => task = Some(PathBuf::from(value)),
            "--tokens" => tokens = Some(number()?),
            "--wall-secs" => wall = Some(Duration::from_secs(number()?)),
            "--seed" => seed = number()?,
            "--work" => work = Some(PathBuf::from(value)),
            "--transcript" => transcript = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    Ok(Args {
        harness: harness.ok_or("--harness is required")?,
        task: task.ok_or("--task is required")?,
        tokens: tokens.ok_or("--tokens is required")?,
        wall: wall.ok_or("--wall-secs is required")?,
        seed,
        work,
        transcript,
    })
}

/// Runs `rsi inner`; returns the report to print.
///
/// # Errors
/// A usage or configuration message, or the run's infrastructure error.
pub fn main(args: &[OsString]) -> Result<String, String> {
    let args = parse(args)?;
    run(&args).map_err(|e| e.to_string())
}

fn run(args: &Args) -> Result<String, RuntimeError> {
    let task = TaskDir::load(&args.task)?;
    let work = args
        .work
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("rsi-inner-{}", std::process::id())));
    let state = work.join("state");
    std::fs::create_dir_all(&state).map_err(|e| RuntimeError::io("creating the work dir", e))?;
    let rsi = std::env::current_exe().map_err(|e| RuntimeError::io("locating rsi", e))?;
    let executor = ProcessExecutor::new(rsi, vec!["__sandbox".into()], state.clone());
    let model = inner_from_env(
        &executor,
        work.join("inner-model"),
        vec![args.task.clone(), state.clone()],
    )
    .map_err(RuntimeError::Model)?;

    let rustc = std::env::var_os("RSI_RUSTC").unwrap_or_else(|| "rustc".into());
    let binary = match build(
        &executor,
        &Toolchain::detect(&rustc)?,
        &args.harness,
        &work.join("build"),
    )? {
        Ok(binary) => binary,
        Err(failure) => return Ok(format!("the harness did not build:\n{}", failure.0)),
    };
    let roots: Vec<PathBuf> = SYSTEM_READ_ROOTS.iter().map(PathBuf::from).collect();
    let runner = SolutionRunner::new(executor.clone(), &roots, &work.join("solutions"))?;
    let public = LocalTask::new(&task, &runner);
    let process = HarnessProcess::new(
        &executor,
        binary,
        &work.join("agent"),
        &[task.root().to_path_buf(), state],
        GRACE,
    )?;
    let harness = SandboxedHarness::new(process, &model, MAX_COMPLETION_TOKENS);
    let budget = Budget::new(args.tokens, args.wall, None)?;
    let seed = Seed::new(args.seed);
    let outcome = harness.run(&public, &budget, seed)?;

    if let Some(path) = &args.transcript {
        std::fs::write(path, &outcome.transcript)
            .map_err(|e| RuntimeError::io("writing the transcript", e))?;
    }
    let usage = outcome.usage;
    let mut report = format!(
        "task {}\ntokens {} of {}\nwall {:.1}s of {}s\n",
        task.manifest().id.as_str(),
        usage.tokens(),
        args.tokens,
        usage.wall.as_secs_f64(),
        args.wall.as_secs()
    );
    match &outcome.submission {
        Some(solution) => {
            let attempt = public.public_score(solution, seed, None)?;
            let score = attempt
                .score
                .map_or_else(|| "buggy".to_owned(), |s| s.get().to_string());
            report.push_str(&format!("public score {score}\n"));
        }
        None => report.push_str(&format!("no submission\nagent log:\n{}\n", outcome.log)),
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_arguments() {
        let args = parse(&os(&[
            "--harness",
            "h",
            "--task",
            "t",
            "--tokens",
            "100",
            "--wall-secs",
            "60",
            "--seed",
            "3",
        ]))
        .expect("valid");
        assert_eq!(args.tokens, 100);
        assert_eq!(args.wall, Duration::from_secs(60));
        assert_eq!(args.seed, 3);
        assert_eq!(args.work, None);
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in [
            &["--task", "t", "--tokens", "1", "--wall-secs", "1"][..],
            &[
                "--harness",
                "h",
                "--task",
                "t",
                "--tokens",
                "x",
                "--wall-secs",
                "1",
            ],
            &["--harness", "h", "--bogus", "1"],
            &["--harness"],
        ] {
            assert!(parse(&os(bad)).is_err(), "{bad:?}");
        }
    }
}
