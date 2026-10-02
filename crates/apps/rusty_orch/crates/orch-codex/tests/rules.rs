//! Saved execpolicy allow rules versus the adapter's `--ignore-rules`.
//!
//! Both tests need the `codex` binary on `PATH`; the second also needs a
//! login and `ORCH_CODEX_REPO`. See the crate README.
//!
//! ```powershell
//! cargo test -p orch-codex --test rules -- --ignored --nocapture
//! ```

mod common;

use std::path::PathBuf;
use std::process::Command;

/// The allow rule a user might have saved after approving a shell command
/// once. `sh -c` as a prefix matches every shell one-liner.
const ALLOW_SH: &str = "prefix_rule(pattern=[\"sh\", \"-c\"], decision=\"allow\")\n";

fn scratch(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("orch-codex-rules-{}-{name}", std::process::id()));
    p
}

/// The hazard, demonstrated offline: with the rule loaded, Codex's own policy
/// evaluator decides `allow` for a command that writes a file. This is what
/// `--ignore-rules` keeps out of every adapter run.
#[test]
#[ignore = "needs the codex binary on PATH"]
fn a_saved_allow_rule_matches_a_write_command() {
    let rules = scratch("allow-sh.rules");
    std::fs::write(&rules, ALLOW_SH).expect("write rules");
    let target = scratch("leak.txt");

    let out = Command::new("codex")
        .args(["execpolicy", "check", "--rules"])
        .arg(&rules)
        .args(["--", "sh", "-c"])
        .arg(format!("echo leak > {}", target.display()))
        .env_remove("OPENAI_API_KEY")
        .output()
        .expect("codex runs");
    let _ = std::fs::remove_file(&rules);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("\"decision\":\"allow\"") || stdout.contains("\"decision\": \"allow\""),
        "{stdout}"
    );
}

/// End to end: an isolated Codex home carrying the allow rule, a model told
/// to run exactly the matching write, and the adapter's argv. The file must
/// not appear. Auth is copied from the real home so the login still works.
#[test]
#[ignore = "needs the codex binary, a ChatGPT login, and ORCH_CODEX_REPO"]
fn adapter_ignores_saved_allow_rules_end_to_end() {
    use orch_codex::CodexAgent;
    use orch_core::board::Board;
    use orch_core::task::{Agent, Plan, Role, TaskSpec};
    use orch_core::{GoalId, Text};
    use orch_dispatch::AgentRunner;

    let repo = std::env::var("ORCH_CODEX_REPO").expect("set ORCH_CODEX_REPO");
    let real_home = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(
                std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .expect("home"),
            )
            .join(".codex")
        });
    let home = scratch("home");
    std::fs::create_dir_all(home.join("rules")).expect("mkdir");
    std::fs::copy(real_home.join("auth.json"), home.join("auth.json"))
        .expect("copy auth.json into the isolated home");
    std::fs::write(home.join("rules").join("default.rules"), ALLOW_SH).expect("write rules");
    let target = scratch("leak.txt");
    let _ = std::fs::remove_file(&target);
    std::env::set_var("CODEX_HOME", &home);

    let mut plan = Plan::new(GoalId::from_raw(1));
    plan.add(TaskSpec {
        role: Role::Research,
        instruction: Text::new(&format!(
            "Run exactly this shell command and report whether it succeeded: sh -c \"echo leak > {}\"",
            target.display()
        ))
        .expect("non-blank"),
        acceptance: vec![Text::new("one finding stating the outcome").expect("non-blank")],
        refs: vec![],
        depends_on: vec![],
        max_calls: std::num::NonZeroU32::new(1).expect("non-zero"),
    })
    .expect("add");
    let board = Board::new(plan.goal());

    let result =
        CodexAgent::new(repo).run(Agent::Codex, plan.tasks().first().expect("card"), &board);
    let leaked = target.exists();
    let _ = std::fs::remove_file(&target);
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        !leaked,
        "the saved allow rule let the write through: {result:?}"
    );
    result.expect("the model still answers after the sandbox denies the write");
}
