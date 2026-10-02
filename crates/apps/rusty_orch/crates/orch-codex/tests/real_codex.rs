//! Runs the real `codex` binary. Ignored by default; see the crate README.
//!
//! ```powershell
//! $env:ORCH_CODEX_REPO = (Get-Location).Path
//! cargo test -p orch-codex --test real_codex -- --ignored --nocapture
//! ```

mod common;

use std::num::NonZeroU32;

use orch_codex::CodexAgent;
use orch_core::board::Board;
use orch_core::task::{Agent, Plan, Role, TaskSpec};
use orch_core::{GoalId, Ref, Text};
use orch_dispatch::AgentRunner;

#[test]
#[ignore = "needs the codex binary, a ChatGPT login, and ORCH_CODEX_REPO"]
fn codex_reads_the_prompt_from_stdin_and_echoes_the_nonce() {
    let repo = std::env::var("ORCH_CODEX_REPO").expect("set ORCH_CODEX_REPO");
    let nonce = format!(
        "ORCH-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let mut plan = Plan::new(GoalId::from_raw(1));
    plan.add(TaskSpec {
        role: Role::Research,
        instruction: Text::new(&format!(
            "Read crates/apps/rusty_orch/AGENTS.md and state its first rule. \
             Include the token {nonce} verbatim in one finding body."
        ))
        .expect("non-blank"),
        acceptance: vec![Text::new("one finding citing the file").expect("non-blank")],
        refs: vec![Ref::Path(
            Text::new("crates/apps/rusty_orch/AGENTS.md").expect("non-blank"),
        )],
        depends_on: vec![],
        max_calls: NonZeroU32::new(1).expect("non-zero"),
    })
    .expect("add");
    let board = Board::new(plan.goal());
    let mut agent = CodexAgent::new(repo);

    // The argv carries no prompt and the model is told nothing but what is
    // on stdin, so a body containing the nonce proves stdin delivery, the
    // schema + last-message path, -C, and the subscription login at once.
    let out = agent
        .run(Agent::Codex, plan.tasks().first().expect("card"), &board)
        .expect("codex produced parseable entries");

    assert!(
        out.iter().any(|o| o.body.as_str().contains(&nonce)),
        "no entry body carried the nonce {nonce}: {out:?}"
    );
}
