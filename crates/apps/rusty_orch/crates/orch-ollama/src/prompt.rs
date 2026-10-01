//! Prompt rendering: the card, the entries it points at, and the spec.

use std::fmt::Write as _;

use orch_core::board::Board;
use orch_core::task::{Role, Task};
use orch_core::Ref;

use crate::parse::{allowed_kinds, MAX_BODY_CHARS, MAX_ENTRIES};

/// Render the prompt for `task`. Includes the instruction, acceptance
/// criteria, refs, and the id and body of every live board entry the card's
/// refs point at (never the whole board), then the output-format spec for
/// the card's role.
pub fn render(task: &Task, board: &Board) -> String {
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
    for r in &spec.refs {
        let Ref::Entry(id) = r else { continue };
        let Some(entry) = board.get(*id).filter(|_| !board.is_superseded(*id)) else {
            continue;
        };
        let _ = writeln!(
            out,
            "{id} [{:?}]: {}",
            entry.content().kind,
            entry.content().body
        );
    }
    out.push_str(&format_spec(spec.role));
    out
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

/// The output contract the parser enforces, phrased for a small model.
fn format_spec(role: Role) -> String {
    let kinds = allowed_kinds(role).join(", ");
    format!(
        "\nOUTPUT FORMAT\n\
         Reply with exactly one JSON object and nothing else:\n\
         {{\"entries\":[{{\"kind\":\"finding\",\"confidence\":\"high\",\"body\":\"...\",\"refs\":[\"E-1\",\"path:src/x.rs\"]}}]}}\n\
         Rules:\n\
         - kind is one of: {kinds}.\n\
         - finding requires confidence: low, medium, or high. Other kinds omit it.\n\
         - body is a short non-blank statement, at most {MAX_BODY_CHARS} characters. Put detail behind refs.\n\
         - refs is an array of strings: E-<n> for a board entry listed above, or path:<repo path>, commit:<hash>, url:<address>.\n\
         - At most {MAX_ENTRIES} entries. At least one.\n"
    )
}
