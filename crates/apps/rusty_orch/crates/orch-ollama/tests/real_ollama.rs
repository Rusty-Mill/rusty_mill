//! Runs the real `ollama` binary. Ignored by default; see the crate README.
//!
//! ```sh
//! ORCH_OLLAMA_MODEL=llama3.2 cargo test -p orch-ollama --test real_ollama -- --ignored
//! ```

use std::num::NonZeroU32;
use std::time::Duration;

use orch_core::board::Board;
use orch_core::task::{Agent, Plan, Role, TaskSpec};
use orch_core::{GoalId, Text};
use orch_dispatch::AgentRunner;
use orch_ollama::OllamaAgent;

#[test]
#[ignore = "needs the ollama binary and ORCH_OLLAMA_MODEL"]
fn ollama_reads_the_prompt_from_stdin_and_echoes_the_nonce() {
    let model = std::env::var("ORCH_OLLAMA_MODEL").expect("set ORCH_OLLAMA_MODEL");
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
            "State one risk of running an unreviewed shell command. \
             Include the token {nonce} verbatim in one finding body."
        ))
        .expect("non-blank"),
        acceptance: vec![Text::new("one finding carrying the token").expect("non-blank")],
        refs: vec![],
        depends_on: vec![],
        max_calls: NonZeroU32::new(1).expect("non-zero"),
    })
    .expect("add");
    let board = Board::new(plan.goal());
    let mut agent = OllamaAgent::new(model, Duration::from_secs(180));

    // The argv carries no prompt and the model is told nothing but what is
    // on stdin, so a body containing the nonce proves stdin delivery under
    // `--format json`; a non-empty reply alone would not (#441).
    let out = agent
        .run(Agent::Local, plan.tasks().first().expect("card"), &board)
        .expect("ollama produced parseable entries");

    assert!(
        out.iter().any(|o| o.body.as_str().contains(&nonce)),
        "no entry body carried the nonce {nonce}: {out:?}"
    );
}
