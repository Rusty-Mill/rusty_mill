use orch_cli::fake::{fixture, research_spec, task, text};
use orch_cli::{format_spec, render};
use orch_core::board::{Author, Confidence, EntryKind, NewEntry};
use orch_core::task::Agent;
use orch_core::{Ref, TaskId};

#[test]
fn render_includes_instruction_acceptance_refs_and_referenced_bodies() {
    let (plan, board, referenced, _) = fixture();
    let prompt = render(task(&plan), &board, &format_spec(task(&plan).spec().role));

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
    let prompt = render(task(&plan), &board, &format_spec(task(&plan).spec().role));

    assert!(!prompt.contains("Unrelated note."));
    assert!(!prompt.contains(&format!("{unreferenced} [")));
}

#[test]
fn render_follows_a_multi_hop_ref_to_the_live_successor() {
    let (plan, mut board, original, _) = fixture();
    let middle = board
        .append(finding("Obsolete middle body.", Some(original), None))
        .expect("middle");
    let live = board
        .append(finding("Current body.", Some(middle), None))
        .expect("live");

    let prompt = render(task(&plan), &board, &format_spec(task(&plan).spec().role));

    assert!(prompt.contains(&format!("{live} [Finding")));
    assert!(prompt.contains("Current body."));
    assert!(!prompt.contains("Plan::start rejects the author."));
    assert!(!prompt.contains("Obsolete middle body."));
}

#[test]
fn render_deduplicates_alias_refs_and_the_card_history() {
    let (fixture_plan, mut board, original, unreferenced) = fixture();
    let card_id = task(&fixture_plan).id();
    let middle = board
        .append(finding("Obsolete middle body.", Some(original), None))
        .expect("middle");
    let live = board
        .append(finding("One live body.", Some(middle), Some(card_id)))
        .expect("live");
    let mut plan = orch_core::task::Plan::new(board.goal());
    plan.add(research_spec(vec![
        Ref::Entry(original),
        Ref::Entry(original),
        Ref::Entry(middle),
        Ref::Entry(live),
    ]))
    .expect("task");

    let prompt = render(task(&plan), &board, &format_spec(task(&plan).spec().role));

    assert_eq!(prompt.matches("One live body.").count(), 1);
    assert_eq!(prompt.matches(&format!("{live} [Finding")).count(), 1);
    assert!(!prompt.contains("THIS CARD SO FAR"));
    assert!(!prompt.contains("Obsolete middle body."));
    assert!(!prompt.contains("Unrelated note."));
    assert!(!prompt.contains(&format!("{unreferenced} [")));
}

#[test]
fn render_ends_with_the_format_spec_for_the_role() {
    let (plan, board, _, _) = fixture();
    let prompt = render(task(&plan), &board, &format_spec(task(&plan).spec().role));

    assert!(prompt.contains("OUTPUT FORMAT"));
    assert!(prompt.contains("kind is one of: finding, question, assumption."));
    assert!(prompt.contains("Never settle a decision"));
    assert!(!prompt.contains("decision,") && !prompt.contains(", decision"));
    assert!(prompt.trim_end().ends_with("At least one."));
}

/// Issue #448: a card resumed after a Question must see the human Answer,
/// even though neither entry is in the card's immutable refs.
#[test]
fn render_includes_the_cards_own_question_and_its_answer() {
    let (plan, mut board, _, unreferenced) = fixture();
    let card = task(&plan);
    let question = board
        .append(NewEntry {
            task: Some(card.id()),
            author: Author::Agent(Agent::Local),
            kind: EntryKind::Question,
            body: text("Which branch is the baseline?"),
            refs: vec![],
            supersedes: None,
        })
        .expect("question");
    // The human's answer is not tagged with the task; it is found via `to`.
    board
        .append(NewEntry {
            task: None,
            author: Author::Human,
            kind: EntryKind::Answer { to: question },
            body: text("Use main as of this morning."),
            refs: vec![],
            supersedes: None,
        })
        .expect("answer");

    let prompt = render(card, &board, &format_spec(card.spec().role));

    assert!(prompt.contains("THIS CARD SO FAR"));
    assert!(prompt.contains("Which branch is the baseline?"));
    assert!(prompt.contains("Use main as of this morning."));
    assert!(
        !prompt.contains("Unrelated note."),
        "other cards' entries stay out"
    );
    assert!(!prompt.contains(&format!("{unreferenced} [")));
}

#[test]
fn render_omits_the_history_block_when_the_card_has_no_entries() {
    let (plan, board, _, _) = fixture();
    let card = task(&plan);
    let prompt = render(card, &board, &format_spec(card.spec().role));
    assert!(!prompt.contains("THIS CARD SO FAR"));
}

fn finding(body: &str, supersedes: Option<orch_core::EntryId>, task: Option<TaskId>) -> NewEntry {
    NewEntry {
        task,
        author: Author::Human,
        kind: EntryKind::Finding {
            confidence: Confidence::High,
        },
        body: text(body),
        refs: vec![],
        supersedes,
    }
}

fn review_of(target: TaskId) -> orch_core::task::TaskSpec {
    orch_core::task::TaskSpec {
        role: orch_core::task::Role::Review { target },
        instruction: text("Review the research for accuracy."),
        acceptance: vec![text("one verdict")],
        refs: vec![],
        depends_on: vec![],
        max_calls: std::num::NonZeroU32::new(1).expect("non-zero"),
    }
}

#[test]
fn a_review_card_sees_every_live_entry_the_target_wrote_without_naming_them() {
    let (mut plan, mut board, _, unreferenced) = fixture();
    let research = task(&plan).id();
    let first = board
        .append(finding("First research finding.", None, Some(research)))
        .expect("first");
    let second = board
        .append(finding("Second research finding.", None, Some(research)))
        .expect("second");
    let review = plan.add(review_of(research)).expect("add review");
    let card = plan.get(review).expect("card");

    let prompt = render(card, &board, &format_spec(card.spec().role));

    assert!(prompt.contains(&format!(
        "UNDER REVIEW (live entries written on task {research}"
    )));
    assert!(prompt.contains(&format!("{first} [Finding")));
    assert!(prompt.contains("First research finding."));
    assert!(prompt.contains(&format!("{second} [Finding")));
    assert!(prompt.contains("Second research finding."));
    assert!(
        !prompt.contains("Unrelated note."),
        "entries off the target stay out"
    );
    assert!(!prompt.contains(&format!("{unreferenced} [")));
}

#[test]
fn a_review_card_sees_the_targets_question_answer_and_resumed_finding() {
    let (mut plan, mut board, _, _) = fixture();
    let research = task(&plan).id();
    let question = board
        .append(NewEntry {
            task: Some(research),
            author: Author::Agent(Agent::Codex),
            kind: EntryKind::Question,
            body: text("Which branch counts?"),
            refs: vec![],
            supersedes: None,
        })
        .expect("question");
    let answer = board
        .append(NewEntry {
            task: Some(research),
            author: Author::Human,
            kind: EntryKind::Answer { to: question },
            body: text("main only."),
            refs: vec![],
            supersedes: None,
        })
        .expect("answer");
    let resumed = board
        .append(finding(
            "After resume: main is the branch.",
            None,
            Some(research),
        ))
        .expect("resumed");
    let review = plan.add(review_of(research)).expect("add review");
    let card = plan.get(review).expect("card");

    let prompt = render(card, &board, &format_spec(card.spec().role));

    for (id, body) in [
        (question, "Which branch counts?"),
        (answer, "main only."),
        (resumed, "After resume: main is the branch."),
    ] {
        assert!(
            prompt.contains(&format!("{id} [")),
            "{id} missing:\n{prompt}"
        );
        assert!(prompt.contains(body), "{body:?} missing:\n{prompt}");
    }
    // The target's entries are listed once, under review, not again as history.
    assert_eq!(prompt.matches(&format!("{resumed} [")).count(), 1);
    assert!(
        !prompt.contains("THIS CARD SO FAR"),
        "the review card has written nothing yet"
    );
}

#[test]
fn a_review_card_with_a_silent_target_says_so() {
    let (mut plan, board, _, _) = fixture();
    let research = task(&plan).id();
    let review = plan.add(review_of(research)).expect("add review");
    let card = plan.get(review).expect("card");

    let prompt = render(card, &board, &format_spec(card.spec().role));

    assert!(prompt.contains("UNDER REVIEW"));
    assert!(prompt.contains("(none)"));
}
