//! Markdown "gap summary" export of a [`TraceReport`], for briefing decks.

use std::fmt::Write as _;

use crate::model::{EdgeState, Maturity, NodeRef, Scenario, TraceReport};

fn escape_cell(text: &str) -> String {
    text.replace('|', "\\|")
}

impl TraceReport {
    /// Renders the report as a Markdown gap summary: the verdict, a
    /// worst-first "what to fund first" table, the domains that already
    /// meet their floor, and the maturity ladder as a footer so the
    /// levels never need explaining separately. `scenario` supplies
    /// outcome descriptions and source names.
    pub fn to_markdown(&self, scenario: &Scenario) -> String {
        let mut md = String::new();
        let _ = writeln!(md, "# Gap summary: {}", self.outcome_name);
        md.push('\n');
        if let Some(outcome) = scenario.outcome(&self.outcome) {
            if !outcome.description.trim().is_empty() {
                let _ = writeln!(md, "> {}", outcome.description.trim());
                md.push('\n');
            }
        }
        let _ = writeln!(md, "**Achievable today:** {}", self.achievable);
        md.push('\n');

        if self.bottlenecks.is_empty() {
            md.push_str("Every required domain meets its maturity floor. No gaps to fund.\n\n");
        } else {
            md.push_str("## What to fund first\n\n");
            md.push_str("| # | Domain | Owner | Today | Needed | Gap | Impact | Data lives in |\n");
            md.push_str("|---|---|---|---|---|---|---|---|\n");
            for (i, b) in self.bottlenecks.iter().enumerate() {
                let impact = match b.state {
                    EdgeState::Blocked => "**Blocking**",
                    EdgeState::Degraded => "Degrading",
                    EdgeState::Missing => "Optional",
                    EdgeState::Satisfied => "-",
                };
                let sources = if b.sources.is_empty() {
                    "_none recorded_".to_string()
                } else {
                    b.sources
                        .iter()
                        .map(|id| {
                            scenario
                                .source(id)
                                .map(|s| format!("{} ({})", escape_cell(&s.name), s.kind.as_str()))
                                .unwrap_or_else(|| escape_cell(id.as_str()))
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let _ = writeln!(
                    md,
                    "| {} | {} | {} | {} | {} | +{} | {} | {} |",
                    i + 1,
                    escape_cell(&b.domain_name),
                    escape_cell(&b.owner),
                    b.current,
                    b.required,
                    b.gap,
                    impact,
                    sources
                );
            }
            md.push('\n');
        }

        let satisfied: Vec<_> = self
            .edges
            .iter()
            .filter(|e| e.state == EdgeState::Satisfied)
            .filter_map(|e| match (&e.from, &e.to) {
                (NodeRef::Domain(d), NodeRef::Outcome(_)) => scenario.domain(d),
                _ => None,
            })
            .collect();
        if !satisfied.is_empty() {
            md.push_str("## Already in place\n\n");
            for d in satisfied {
                let _ = writeln!(md, "- {} -- {} ({})", d.name, d.maturity, d.owner);
            }
            md.push('\n');
        }

        md.push_str("---\n\n_Maturity ladder: ");
        let ladder: Vec<String> = Maturity::ALL.iter().map(|m| m.to_string()).collect();
        md.push_str(&ladder.join(" · "));
        md.push_str("._\n");
        md
    }
}
