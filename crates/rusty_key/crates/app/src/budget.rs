//! Token budget + 3-tier compaction (PRD 06 / Phase 8). The [`TokenBudget`]
//! tracks a **line-item** estimate (system + recall + task + tool schemas +
//! history) against the model's context window and decides which compaction
//! tier a turn triggers. The pure message surgery lives here; the session
//! performs the LLM summarisation for the `session`/`full` tiers.

/// One message in the in-session conversation transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    /// `user`, `assistant`, `summary`, or `marker`.
    pub role: String,
    /// The message text.
    pub content: String,
}

impl Msg {
    /// A user turn.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
    /// An assistant reply.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
    /// A summary inserted by compaction.
    pub fn summary(content: impl Into<String>) -> Self {
        Self {
            role: "summary".into(),
            content: content.into(),
        }
    }
}

/// The compaction tier a usage level triggers (escalating).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Below the warn threshold — no action.
    None,
    /// Early pressure: trim only the very oldest turn-pairs (a light micro that
    /// keeps more recent context); no LLM call. The finer tier added in P4.
    Warn,
    /// Drop oldest turn-pairs; no LLM call.
    Micro,
    /// Summarise the oldest half via the model.
    Session,
    /// Summarise everything; reset history to one summary.
    Full,
}

/// How far below the `micro` threshold the `Warn` tier opens (fraction of the
/// context window). With the default `micro = 0.80`, `Warn` covers `[0.70, 0.80)`.
pub const WARN_BAND: f64 = 0.10;

/// Heuristic token estimate (~4 chars/token). Cheap and provider-agnostic; the
/// real provider usage refines `session_total_tokens` when available.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Flatten a transcript to `role: content` lines (the kernel prompt history).
pub fn flatten(history: &[Msg]) -> String {
    history
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Tracks the token budget and compaction thresholds for a session.
#[derive(Debug, Clone)]
pub struct TokenBudget {
    /// The model's context window, in tokens.
    pub context_limit: usize,
    /// Micro / session / full thresholds, as fractions of `context_limit`.
    pub micro: f64,
    pub session: f64,
    pub full: f64,
    /// Line-item estimate of the most recent turn's prompt.
    pub used_tokens: usize,
    /// Cumulative tokens consumed this session.
    pub session_total_tokens: u64,
    /// How many compactions have fired this session.
    pub compaction_count: usize,
    /// Calibration factor `real / estimate` learned from provider usage (P4):
    /// the char/4 heuristic is corrected toward the provider's real token counts
    /// so tiers fire on real tokens, not raw length. `1.0` until the first turn
    /// with reported usage (e.g. the offline fake never reports any).
    pub calibration: f64,
}

impl TokenBudget {
    /// Build with explicit thresholds.
    pub fn new(context_limit: usize, micro: f64, session: f64, full: f64) -> Self {
        Self {
            context_limit,
            micro,
            session,
            full,
            used_tokens: 0,
            session_total_tokens: 0,
            compaction_count: 0,
            calibration: 1.0,
        }
    }

    /// Sum the per-turn line items into a token estimate. This — not history
    /// length alone — is what drives the thresholds (DoD).
    pub fn line_items(
        &self,
        system: &str,
        recall: &str,
        task: &str,
        tool_schemas: &str,
        history: &[Msg],
    ) -> usize {
        estimate_tokens(system)
            + estimate_tokens(recall)
            + estimate_tokens(task)
            + estimate_tokens(tool_schemas)
            + estimate_tokens(&flatten(history))
    }

    /// Which tier `used` tokens triggers. `used` should already be
    /// calibration-corrected ([`Self::calibrated`]) so the bands fire on real
    /// tokens. The `Warn` band sits just below `micro` (see [`WARN_BAND`]).
    pub fn tier_for(&self, used: usize) -> Tier {
        let frac = used as f64 / (self.context_limit.max(1) as f64);
        if frac >= self.full {
            Tier::Full
        } else if frac >= self.session {
            Tier::Session
        } else if frac >= self.micro {
            Tier::Micro
        } else if frac >= (self.micro - WARN_BAND).max(0.0) {
            Tier::Warn
        } else {
            Tier::None
        }
    }

    /// Correct a raw line-item estimate by the learned calibration factor (P4).
    pub fn calibrated(&self, estimate: usize) -> usize {
        ((estimate as f64) * self.calibration).round() as usize
    }

    /// Record a turn's usage and, when the provider reported real input tokens,
    /// nudge the calibration factor toward `real / estimate` (P4). `used_tokens`
    /// becomes the real count when known, else the calibrated estimate, so
    /// `/cost` and the next turn's decision both reflect real tokens.
    pub fn observe_turn(&mut self, estimate: usize, real_input: Option<usize>) {
        if let Some(real) = real_input {
            if estimate > 0 && real > 0 {
                // Exponential moving average, clamped so one outlier can't make
                // the heuristic wildly over- or under-count.
                let ratio = real as f64 / estimate as f64;
                self.calibration = (0.5 * self.calibration + 0.5 * ratio).clamp(0.25, 4.0);
            }
        }
        let resolved = real_input.unwrap_or_else(|| self.calibrated(estimate));
        self.used_tokens = resolved;
        self.session_total_tokens += resolved as u64;
    }

    /// Record the resolved per-turn usage (estimate path; no provider usage).
    pub fn record_usage(&mut self, used: usize) {
        self.observe_turn(used, None);
    }

    /// Fraction of the window currently used (for `/cost`).
    pub fn fraction(&self) -> f64 {
        self.used_tokens as f64 / (self.context_limit.max(1) as f64)
    }
}

/// Drop oldest turn-pairs (a `user`+`assistant` pair) until the history is at
/// most `keep_pairs` pairs, leaving a `[compacted N turns]` marker. Returns the
/// number of messages dropped. No LLM call.
pub fn micro_compact(history: &mut Vec<Msg>, keep_pairs: usize) -> usize {
    // Count droppable leading messages, preserving the last `keep_pairs` pairs.
    let keep_msgs = keep_pairs * 2;
    if history.len() <= keep_msgs {
        return 0;
    }
    // Don't drop a leading existing marker count; recompute fresh.
    let drop_n = history.len() - keep_msgs;
    let removed: Vec<Msg> = history.drain(0..drop_n).collect();
    let dropped = removed.len();
    // Carry forward the count from any earlier marker so the tally is cumulative.
    let prior_turns = removed
        .iter()
        .filter(|m| m.role == "marker")
        .filter_map(|m| {
            m.content
                .strip_prefix("[compacted ")
                .and_then(|s| s.split(' ').next())
                .and_then(|n| n.parse::<usize>().ok())
        })
        .sum::<usize>();
    let turns = prior_turns + removed.iter().filter(|m| m.role == "user").count();
    history.insert(
        0,
        Msg {
            role: "marker".into(),
            content: format!("[compacted {turns} turns]"),
        },
    );
    dropped
}

/// History takes precedence over recall: drop any recall entry whose memory
/// title already appears verbatim in the live transcript, so the same content
/// is not sent twice (PRD 08 backlog item). Returns the filtered recall block.
pub fn dedup_recall_block(recall_block: &str, history_text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in recall_block.lines() {
        let is_entry = line.starts_with("- ");
        let is_cont = line.starts_with("  ");
        if is_entry {
            // Title is between "] " and the first ": " of the entry line.
            let title = line
                .split_once("] ")
                .and_then(|(_, rest)| rest.split_once(": "))
                .map(|(t, _)| t)
                .unwrap_or("");
            skipping = !title.is_empty() && history_text.contains(title);
            if skipping {
                continue;
            }
        } else if is_cont {
            if skipping {
                continue;
            }
        } else {
            skipping = false;
        }
        out.push(line);
    }
    // If only the header survives, the block is empty of content.
    if out.iter().all(|l| !l.starts_with("- ")) {
        return String::new();
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(n: usize) -> Vec<Msg> {
        let mut v = Vec::new();
        for i in 0..n {
            v.push(Msg::user(format!("u{i}")));
            v.push(Msg::assistant(format!("a{i}")));
        }
        v
    }

    #[test]
    fn tiers_escalate_with_usage() {
        let b = TokenBudget::new(1000, 0.80, 0.90, 0.95);
        assert_eq!(b.tier_for(690), Tier::None);
        // Warn band: ≈[micro - WARN_BAND, micro) = ≈[0.70, 0.80). Assert inside the
        // band, not on the float-fragile lower edge.
        assert_eq!(b.tier_for(750), Tier::Warn);
        assert_eq!(b.tier_for(790), Tier::Warn);
        assert_eq!(b.tier_for(800), Tier::Micro);
        assert_eq!(b.tier_for(900), Tier::Session);
        assert_eq!(b.tier_for(950), Tier::Full);
        assert_eq!(b.tier_for(1200), Tier::Full);
    }

    #[test]
    fn real_usage_calibrates_the_estimate() {
        let mut b = TokenBudget::new(1000, 0.80, 0.90, 0.95);
        assert_eq!(b.calibration, 1.0);
        // Provider reports 2× our estimate → calibration rises toward 2.0.
        b.observe_turn(100, Some(200));
        assert!(b.calibration > 1.0, "calibration should rise: {}", b.calibration);
        // used_tokens is the real count when known.
        assert_eq!(b.used_tokens, 200);
        // A subsequent estimate is scaled up by the learned factor.
        assert!(b.calibrated(100) > 100);
    }

    #[test]
    fn calibration_is_clamped_and_ignores_missing_usage() {
        let mut b = TokenBudget::new(1000, 0.80, 0.90, 0.95);
        // No provider usage (the fake): calibration stays 1.0, estimate is used.
        b.observe_turn(120, None);
        assert_eq!(b.calibration, 1.0);
        assert_eq!(b.used_tokens, 120);
        // Absurd ratios are clamped so one outlier can't blow up the estimate.
        for _ in 0..20 {
            b.observe_turn(1, Some(1_000_000));
        }
        assert!(b.calibration <= 4.0);
    }

    #[test]
    fn line_items_sum_all_sources_not_just_history() {
        let b = TokenBudget::new(1000, 0.8, 0.9, 0.95);
        let used = b.line_items("system", "recall", "task", "schemas", &pairs(1));
        // Each source contributes; dropping history alone never zeroes it.
        assert!(used >= estimate_tokens("system"));
        assert!(used > b.line_items("", "", "", "", &pairs(1)));
    }

    #[test]
    fn micro_compact_keeps_last_pairs_and_marks() {
        let mut h = pairs(5); // 10 msgs
        let dropped = micro_compact(&mut h, 2); // keep 2 pairs = 4 msgs + marker
        assert_eq!(dropped, 6);
        assert_eq!(h.first().unwrap().role, "marker");
        assert_eq!(h.first().unwrap().content, "[compacted 3 turns]");
        assert_eq!(h.len(), 5); // marker + 4 kept
    }

    #[test]
    fn micro_compact_noop_when_small() {
        let mut h = pairs(2);
        assert_eq!(micro_compact(&mut h, 2), 0);
        assert_eq!(h.len(), 4);
    }

    #[test]
    fn micro_compact_accumulates_prior_marker_count() {
        let mut h = vec![Msg {
            role: "marker".into(),
            content: "[compacted 3 turns]".into(),
        }];
        h.extend(pairs(3)); // + 6 msgs
        let dropped = micro_compact(&mut h, 1); // keep 1 pair
        assert!(dropped > 0);
        // 3 prior + the freshly-dropped user turns.
        assert_eq!(h.first().unwrap().content, "[compacted 5 turns]");
    }

    #[test]
    fn dedup_drops_entries_already_in_history() {
        let block = "## Relevant memory\n- [fact] Auth uses JWT: details here\n- [skill] Retry logic: more\n";
        let history = "user: remind me\nassistant: Auth uses JWT tokens";
        let out = dedup_recall_block(block, history);
        assert!(!out.contains("Auth uses JWT"));
        assert!(out.contains("Retry logic"));
    }

    #[test]
    fn dedup_returns_empty_when_all_duplicated() {
        let block = "## Relevant memory\n- [fact] Auth uses JWT: details\n";
        let history = "assistant: Auth uses JWT";
        assert_eq!(dedup_recall_block(block, history), "");
    }
}
