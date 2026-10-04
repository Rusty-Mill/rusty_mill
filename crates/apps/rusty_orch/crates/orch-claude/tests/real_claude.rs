//! Runs the real `claude` binary. Ignored by default; see the crate README.
//!
//! ```sh
//! ORCH_REPO=$(pwd) cargo test -p orch-claude --test real_claude -- --ignored --nocapture
//! ```

use std::num::NonZeroU32;

use orch_claude::ClaudeAgent;
use orch_core::board::Board;
use orch_core::task::{Agent, Plan, Role, TaskSpec};
use orch_core::{GoalId, Ref, Text};
use orch_dispatch::AgentRunner;

#[test]
#[ignore = "needs the claude binary, a Claude login, and ORCH_REPO"]
fn claude_reads_the_prompt_from_stdin_and_echoes_the_nonce() {
    let repo = std::env::var("ORCH_REPO").expect("set ORCH_REPO");
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

    let out = ClaudeAgent::new(repo)
        .run(Agent::Claude, plan.tasks().first().expect("card"), &board)
        .expect("claude answers");

    assert!(
        out.iter().any(|o| o.body.as_str().contains(&nonce)),
        "no entry carried the nonce: {out:?}"
    );
}
