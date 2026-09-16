//! Terminal rendering.
//!
//! Monochrome by design, matching the product it reimplements: dim for
//! metadata, bold for the thing you are looking for, and one accent reserved
//! for the "found by meaning" label — the one piece of information a user
//! would otherwise mistake for a bug.

use inventory_core::format;
use inventory_core::{MatchedVia, SearchHit, SearchResponse, SourceState, SourceStatus, Stats};

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const ACCENT: &str = "\x1b[36m";
pub const WARN: &str = "\x1b[33m";

pub fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some()
}

pub fn paint(code: &str, text: &str) -> String {
    if no_color() {
        text.to_string()
    } else {
        format!("{code}{text}{RESET}")
    }
}

/// Render `[matched]` markers from an FTS snippet as bold.
fn highlight(snippet: &str) -> String {
    let flat = snippet.replace('\n', " ");
    if no_color() {
        return flat;
    }
    let mut out = String::new();
    let mut rest = flat.as_str();
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        out.push_str(&rest[..start]);
        out.push_str(&paint(BOLD, &rest[start + 1..start + end]));
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

pub fn search_results(response: &SearchResponse, query: &str) {
    if response.hits.is_empty() {
        println!("{}", paint(DIM, &format!("No results for “{query}”.")));
        return;
    }

    for (i, hit) in response.hits.iter().enumerate() {
        print_hit(i + 1, hit, response.semantic_available);
    }

    println!();
    let summary = format!(
        "{} of {} candidates · model {}{}",
        response.hits.len(),
        response.total_candidates,
        response.semantic_model,
        if response.semantic_available {
            ""
        } else {
            " (lexical fallback — index more conversations to enable meaning search)"
        }
    );
    println!("{}", paint(DIM, &summary));
}

fn print_hit(n: usize, hit: &SearchHit, semantic_available: bool) {
    let c = &hit.conversation;
    let mut meta = vec![
        c.source.display_name().to_string(),
        format!("{} messages", c.message_count),
    ];
    if let Some(b) = &c.git_branch {
        meta.push(b.clone());
    }
    meta.push(format::relative(c.updated_at));

    // Only claim "meaning" when a model that actually models meaning ran.
    let tag = match hit.matched_via {
        MatchedVia::Meaning if semantic_available => Some(paint(ACCENT, "  ⟡ found by meaning")),
        MatchedVia::Meaning => Some(paint(DIM, "  ⟡ found by similarity")),
        _ => None,
    };

    println!(
        "{} {}{}",
        paint(DIM, &format!("{n:>2}.")),
        paint(BOLD, &c.title),
        tag.unwrap_or_default()
    );
    println!("    {}", paint(DIM, &meta.join(" · ")));
    if !hit.snippet.is_empty() {
        println!("    {}", highlight(&hit.snippet));
    }
    println!("    {}", paint(DIM, &format!("id {}", c.id)));
    println!();
}

pub fn sources(statuses: &[SourceStatus]) {
    println!("{}", paint(BOLD, "Sources"));
    println!();
    for s in statuses {
        let (mark, state) = match s.state {
            SourceState::Ok => (paint(ACCENT, "●"), "indexed".to_string()),
            SourceState::Frozen => (paint(WARN, "●"), "frozen".to_string()),
            SourceState::Absent => (paint(DIM, "○"), "not installed".to_string()),
        };
        println!(
            "  {mark} {:<14} {}",
            s.source.display_name(),
            paint(DIM, &state)
        );

        if s.conversation_count > 0 {
            println!(
                "      {}",
                paint(
                    DIM,
                    &format!(
                        "{} conversations · {} messages",
                        s.conversation_count, s.message_count
                    )
                )
            );
        }
        if s.state == SourceState::Frozen {
            // The last successful read is the number that matters when a
            // source goes unreadable.
            let last = s
                .last_ok_at
                .map(|t| format!("last read cleanly {}", format::relative(t)))
                .unwrap_or_else(|| "never read cleanly".into());
            println!("      {}", paint(WARN, &last));
            println!(
                "      {}",
                paint(
                    DIM,
                    "history already indexed is retained and still searchable; retried on next index"
                )
            );
            if let Some(e) = &s.last_error {
                println!("      {}", paint(DIM, &truncate(e, 100)));
            }
        }
    }
}

pub fn stats(stats: &Stats) {
    println!("{}", paint(BOLD, "Index"));
    println!();
    row("Conversations", &stats.conversations.to_string());
    row("Messages", &stats.messages.to_string());
    row("On disk", &format::bytes(stats.index_bytes as i64));
    row(
        "Encrypted at rest",
        &if stats.encrypted {
            format!("yes · {:.4} bits/byte entropy", stats.entropy_bits_per_byte)
        } else {
            "no".into()
        },
    );
    row("Retention", stats.retention.label());
    row(
        "Semantic model",
        &format!(
            "{}{}",
            stats.embedding_model,
            if stats.semantic_available {
                ""
            } else {
                " (lexical fallback)"
            }
        ),
    );
    row(
        "Embedded",
        &format!("{} conversations", stats.embedded_conversations),
    );
    row(
        "Last indexed",
        &stats
            .last_index_at
            .map(format::relative)
            .unwrap_or_else(|| "never".into()),
    );
    row("Captured notes", &stats.notes.to_string());
    row(
        "Scratchpad",
        &if stats.scratchpad_enabled {
            format!("on · {} clips", stats.clips)
        } else {
            "off".into()
        },
    );

    println!();
    println!("{}", paint(BOLD, "By source"));
    println!();
    for (source, n) in &stats.per_source {
        row(source.display_name(), &n.to_string());
    }
}

pub fn row(label: &str, value: &str) {
    println!("  {:<22} {}", paint(DIM, label), value);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}
