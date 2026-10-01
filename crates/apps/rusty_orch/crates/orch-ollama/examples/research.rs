//! One research card, end to end, on a local Ollama model.
//!
//! ```sh
//! ORCH_OLLAMA_MODEL=llama3.2 cargo run -p orch-ollama --example research -- "How does Plan::start prevent self-review?"
//! ```

use std::num::NonZeroU32;
use std::time::Duration;

use orch_core::board::Board;
use orch_core::goal::{Goal, GoalDraft, StopRule};
use orch_core::task::{Agent, Plan, Role, TaskSpec};
use orch_core::{GoalId, Text};
use orch_dispatch::{Dispatcher, Ledger, Routing, RoutingConfig};
use orch_ollama::OllamaAgent;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let question = std::env::args()
        .nth(1)
        .ok_or("usage: research \"<question>\"")?;
    let model = std::env::var("ORCH_OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2".to_owned());

    let goal = Goal::try_from(GoalDraft {
        outcome: Some(question.clone()),
        done_when: vec!["at least one finding on the board".to_owned()],
        out_of_scope: Some(vec!["code changes".to_owned()]),
        wall_clock_secs: Some(600),
        max_calls: Some(3),
        stop: Some(StopRule::Checkpoint),
        ..GoalDraft::default()
    })?;
    let id = GoalId::from_raw(1);
    let mut plan = Plan::new(id);
    plan.add(TaskSpec {
        role: Role::Research,
        instruction: Text::new(&question).ok_or("blank question")?,
        acceptance: vec![Text::new("one finding with a ref").ok_or("blank")?],
        refs: vec![],
        depends_on: vec![],
        max_calls: NonZeroU32::new(2).ok_or("zero")?,
    })?;
    let mut board = Board::new(id);
    let mut ledger = Ledger::new();

    let routing = Routing::try_from(RoutingConfig {
        research: Agent::Local,
        design: Agent::Local,
        implement: Agent::Local,
        triage: Agent::Local,
        reviewers: vec![Agent::Local, Agent::Claude],
    })?;
    let mut dispatcher =
        Dispatcher::new(routing, OllamaAgent::new(model, Duration::from_secs(180)));

    let outcome = dispatcher.run(&goal, &mut plan, &mut board, &mut ledger)?;
    println!("outcome: {outcome:?} after {} call(s)\n", ledger.calls());
    for entry in board.live() {
        let c = entry.content();
        println!("{} [{:?}] {:?}", entry.id(), c.kind, c.author);
        println!("  {}", c.body);
        for r in &c.refs {
            println!("  -> {r:?}");
        }
    }
    Ok(())
}
