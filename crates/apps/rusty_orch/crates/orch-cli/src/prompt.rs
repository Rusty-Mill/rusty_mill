//! Prompt rendering: the card, the entries it points at, and the adapter's
//! format-spec footer.

use std::collections::HashSet;
use std::fmt::Write as _;

use orch_core::board::{Board, Entry, EntryKind};
use orch_core::task::{Role, Task};
use orch_core::{EntryId, Ref, TaskId};

use crate::parse::{allowed_kinds, MAX_BODY_CHARS, MAX_ENTRIES};

/// Render the prompt for `task`. Includes the instruction, acceptance
/// criteria, refs, the id and body of every live board entry the card's
/// refs point at (never the whole board), for a review card every live
/// entry the reviewed task wrote and the answers to them (resolved at
/// render time, so the author need not guess entry ids), the card's own
/// earlier entries and the answers to them (so a card resumed after a
/// `Question` sees its `Answer`), then `footer`: the adapter's
/// output-format spec, normally [`format_spec`] plus any adapter lines.
pub fn render(task: &Task, board: &Board, footer: &str) -> String {
    let spec = task.spec();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "You are a {} agent. Task {}.",
        role_name(spec.role),
        task.id()
    );
    let _ = writeln!(out, "\nINSTRUCTION\n{}", spec.instruction);
    out.push_str("\nACCEPTANCE\n");
    for a in &spec.acceptance {
        let _ = writeln!(out, "- {a}");
    }
    out.push_str("\nREFS\n");
    for r in &spec.refs {
        let _ = writeln!(out, "- {}", ref_line(r));
    }
    out.push_str("\nCONTEXT (live board entries referenced above)\n");
    let mut shown = HashSet::new();
    for r in &spec.refs {
        let Ref::Entry(id) = r else { continue };
        let Some(entry) = live_successor(board, *id) else {
            continue;
        };
        if !shown.insert(entry.id()) {
            continue;
        }
        let _ = writeln!(
            out,
            "{} [{:?}]: {}",
            entry.id(),
            entry.content().kind,
            entry.content().body
        );
    }
    if let Role::Review { target } = spec.role {
        let output = written_on(board, target, &shown);
        let _ = writeln!(
            out,
            "\nUNDER REVIEW (live entries written on task {target}, and answers to them)"
        );
        if output.is_empty() {
            out.push_str("(none)\n");
        }
        for entry in output {
            shown.insert(entry.id());
            let _ = writeln!(
                out,
                "{} [{:?}]: {}",
                entry.id(),
                entry.content().kind,
                entry.content().body
            );
        }
    }
    let history = written_on(board, task.id(), &shown);
    if !history.is_empty() {
        out.push_str(
            "\nTHIS CARD SO FAR (your earlier entries on this card, and answers to them)\n",
        );
        for entry in history {
            let _ = writeln!(
                out,
                "{} [{:?}]: {}",
                entry.id(),
                entry.content().kind,
                entry.content().body
            );
        }
    }
    out.push_str(footer);
    out
}

/// Live entries written on `task`, plus live answers to any of them, minus
/// anything already shown. Board order. Serves both the reviewed task's
/// output and the card's own history.
fn written_on<'b>(board: &'b Board, task: TaskId, shown: &HashSet<EntryId>) -> Vec<&'b Entry> {
    let own: Vec<EntryId> = board
        .live()
        .filter(|e| e.content().task == Some(task))
        .map(Entry::id)
        .collect();
    board
        .live()
        .filter(|e| {
            let answers_own =
                matches!(e.content().kind, EntryKind::Answer { to } if own.contains(&to));
            (own.contains(&e.id()) || answers_own) && !shown.contains(&e.id())
        })
        .collect()
}

/// Follow the board's linear supersession links to the current entry.
fn live_successor(board: &Board, mut id: EntryId) -> Option<&Entry> {
    board.get(id)?;
    while let Some(successor) = board
        .entries()
        .iter()
        .find(|entry| entry.content().supersedes == Some(id))
    {
        id = successor.id();
    }
    board.get(id)
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Research => "research",
        Role::Design => "design",
        Role::Implement => "implementation",
        Role::Triage => "triage",
        Role::Review { .. } => "review",
    }
}

fn ref_line(r: &Ref) -> String {
    match r {
        Ref::Path(t) => format!("path:{t}"),
        Ref::Commit(t) => format!("commit:{t}"),
        Ref::Url(t) => format!("url:{t}"),
        Ref::Entry(id) => id.to_string(),
    }
}

/// The output contract [`crate::parse`] enforces, phrased for a small model.
pub fn format_spec(role: Role) -> String {
    let kinds = allowed_kinds(role).join(", ");
    format!(
        "\nOUTPUT FORMAT\n\
         Reply with exactly one JSON object and nothing else:\n\
         {{\"entries\":[{{\"kind\":\"finding\",\"confidence\":\"high\",\"body\":\"...\",\"refs\":[\"E-1\",\"path:src/x.rs\"]}}]}}\n\
         Rules:\n\
         - kind is one of: {kinds}.\n\
         - finding requires confidence: low, medium, or high. Other kinds omit it.\n\
         - review requires verdict: approve or changes_requested, and a review card includes exactly one review entry.\n\
         - body is a short non-blank statement, at most {MAX_BODY_CHARS} characters. Put detail behind refs.\n\
         - refs is an array of strings: E-<n> for a board entry listed above, or path:<repo path>, commit:<hash>, url:<address>.\n\
         - Never settle a decision; propose it as a finding.\n\
         - At most {MAX_ENTRIES} entries. At least one.\n"
    )
}
