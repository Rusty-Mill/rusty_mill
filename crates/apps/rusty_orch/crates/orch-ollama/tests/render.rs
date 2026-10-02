mod common;

use common::{fixture, task};
use orch_ollama::render;

#[test]
fn render_ends_with_the_format_spec_for_the_role() {
    let (plan, board, _, _) = fixture();
    let prompt = render(task(&plan), &board);

    assert!(prompt.contains("OUTPUT FORMAT"));
    assert!(prompt.contains("kind is one of: finding, question, assumption."));
    assert!(prompt.contains("Never settle a decision"));
    assert!(!prompt.contains("decision,") && !prompt.contains(", decision"));
    assert!(prompt.trim_end().ends_with("At least one."));
}
