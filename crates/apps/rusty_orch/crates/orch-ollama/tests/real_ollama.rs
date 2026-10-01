//! Runs the real `ollama` binary. Ignored by default; see the crate README.
//!
//! ```sh
//! ORCH_OLLAMA_MODEL=llama3.2 cargo test -p orch-ollama --test real_ollama -- --ignored
//! ```

mod common;

use std::time::Duration;

use common::{fixture, task};
use orch_core::task::Agent;
use orch_dispatch::AgentRunner;
use orch_ollama::OllamaAgent;

#[test]
#[ignore = "needs the ollama binary and ORCH_OLLAMA_MODEL"]
fn ollama_reads_the_prompt_from_stdin_and_returns_entries() {
    let model = std::env::var("ORCH_OLLAMA_MODEL").expect("set ORCH_OLLAMA_MODEL");
    let (plan, board, _, _) = fixture();
    let mut agent = OllamaAgent::new(model, Duration::from_secs(180));

    // The argv carries no prompt, so any entry at all proves the model read
    // the card from stdin under `--format json`.
    let out = agent
        .run(Agent::Local, task(&plan), &board)
        .expect("ollama produced parseable entries");

    assert!(!out.is_empty());
}
