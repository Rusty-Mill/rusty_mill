//! Codex researches, a local Ollama model reviews, on one board.
//!
//! ```powershell
//! $env:ORCH_OLLAMA_MODEL = "llama3.2"
//! cargo run -p orch-codex --example research_review -- "How does Plan::start prevent self-review?"
//! ```
//! Runs from the repository root; `ORCH_CODEX_REPO` overrides the working
//! directory Codex reads.

use std::num::NonZeroU32;
use std::time::Duration;

use orch_codex::CodexAgent;
use orch_core::board::Board;
use orch_core::goal::{Goal, GoalDraft, StopRule};
use orch_core::task::{Agent, Plan, Role, Task, TaskSpec, TaskState};
use orch_core::{GoalId, Ref, Text};
use orch_dispatch::{AgentError, AgentRunner, Dispatcher, Ledger, Output, Routing, RoutingConfig};
use orch_ollama::OllamaAgent;

/// One runner per agent; the dispatcher only ever hands us the agent it routed.
struct Agents {
    codex: CodexAgent,
    local: OllamaAgent,
}

impl AgentRunner for Agents {
    fn run(&mut self, agent: Agent, task: &Task, board: &Board) -> Result<Vec<Output>, AgentError> {
        match agent {
            Agent::Codex => self.codex.run(agent, task, board),
            Agent::Local => self.local.run(agent, task, board),
            other => Err(AgentError(format!("no adapter for {other:?}"))),
        }
    }
}

fn text(s: &str) -> Result<Text, Box<dyn std::error::Error>> {
    Ok(Text::new(s).ok_or("blank text")?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let question = std::env::args()
        .nth(1)
        .ok_or("usage: research_review \"<question>\"")?;
    let model = std::env::var("ORCH_OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2".to_owned());
    let repo = std::env::var("ORCH_CODEX_REPO")
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_dir()?);

    let goal = Goal::try_from(GoalDraft {
        outcome: Some(question.clone()),
        done_when: vec!["a reviewed finding on the board".to_owned()],
        out_of_scope: Some(vec!["code changes".to_owned()]),
        wall_clock_secs: Some(1200),
        max_calls: Some(4),
        stop: Some(StopRule::Checkpoint),
        ..GoalDraft::default()
    })?;
    let id = GoalId::from_raw(1);
    let mut plan = Plan::new(id);
    let mut board = Board::new(id);
    let mut ledger = Ledger::new();
    let routing = Routing::try_from(RoutingConfig {
        research: Agent::Codex,
        design: Agent::Codex,
        implement: Agent::Codex,
        triage: Agent::Local,
        reviewers: vec![Agent::Local, Agent::Codex],
    })?;
    let mut dispatcher = Dispatcher::new(
        routing,
        Agents {
            codex: CodexAgent::new(repo),
            local: OllamaAgent::new(model, Duration::from_secs(180)),
        },
    );

    // Round 1: Codex researches.
    let research = plan.add(TaskSpec {
        role: Role::Research,
        instruction: text(&question)?,
        acceptance: vec![text("one finding with a path: ref")?],
        refs: vec![Ref::Path(text(
            "crates/apps/rusty_orch/crates/orch-core/src/task.rs",
        )?)],
        depends_on: vec![],
        max_calls: NonZeroU32::new(2).ok_or("zero")?,
    })?;
    let first = dispatcher.run(&goal, &mut plan, &mut board, &mut ledger)?;
    println!("round 1: {first:?}");

    // Round 2: a Local reviewer reads what Codex wrote and gives a verdict.
    let outputs = match plan.get(research).map(Task::state) {
        Some(TaskState::Done { outputs, .. }) => outputs.clone(),
        other => return Err(format!("research did not finish: {other:?}").into()),
    };
    plan.add(TaskSpec {
        role: Role::Review { target: research },
        instruction: text("Review the referenced findings for accuracy against the code. Approve or request changes.")?,
        acceptance: vec![text("one review entry with a verdict")?],
        refs: outputs.into_iter().map(Ref::Entry).collect(),
        depends_on: vec![],
        max_calls: NonZeroU32::new(2).ok_or("zero")?,
    })?;
    let second = dispatcher.run(&goal, &mut plan, &mut board, &mut ledger)?;
    println!("round 2: {second:?} after {} call(s)\n", ledger.calls());

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
