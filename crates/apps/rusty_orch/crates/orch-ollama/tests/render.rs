mod common;

use common::{fixture, task};
use orch_ollama::render;

#[test]
fn render_includes_instruction_acceptance_refs_and_referenced_bodies() {
    let (plan, board, referenced, _) = fixture();
    let prompt = render(task(&plan), &board);

    assert!(prompt.contains("Explain how Plan::start enforces no self-review."));
    assert!(prompt.contains("- cites the function"));
    assert!(prompt.contains("- one finding minimum"));
    assert!(prompt.contains("- path:crates/orch-core/src/task.rs"));
    assert!(prompt.contains(&format!("- {referenced}")));
    assert!(prompt.contains(&format!("{referenced} [Finding")));
    assert!(prompt.contains("Plan::start rejects the author."));
}

#[test]
fn render_excludes_unreferenced_entries() {
    let (plan, board, _, unreferenced) = fixture();
    let prompt = render(task(&plan), &board);

    assert!(!prompt.contains("Unrelated note."));
    assert!(!prompt.contains(&format!("{unreferenced} [")));
}

#[test]
fn render_ends_with_the_format_spec_for_the_role() {
    let (plan, board, _, _) = fixture();
    let prompt = render(task(&plan), &board);

    assert!(prompt.contains("OUTPUT FORMAT"));
    assert!(prompt.contains("kind is one of: finding, question, assumption."));
    assert!(prompt.trim_end().ends_with("At least one."));
}
