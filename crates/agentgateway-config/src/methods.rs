//! Which MCP methods a guardrail processor runs for.
//!
//! A processor's `methods` map is an allow-list from pattern to [`Phase`]. A
//! method can match several patterns at once, so the order below decides which
//! one wins. It is upstream's, and it is worth stating because a config that
//! resolves differently here than there is a config that behaves differently
//! in production than in staging.

use std::collections::BTreeMap;

use crate::Phase;

/// The MCP methods this gateway's federation actually serves.
///
/// Lives here rather than in `agentgateway-mcp` so `Config::lint` can report a
/// processor keyed on a method that never arrives. The MCP crate imports this
/// list rather than keeping its own, so the two cannot drift apart.
pub const MCP_SERVED_METHODS: &[&str] = &[
    "prompts/get",
    "prompts/list",
    "resources/list",
    "resources/read",
    "resources/templates/list",
    "tools/call",
    "tools/list",
];

/// Whether a `methods` key is a pattern [`resolve`] can ever match.
///
/// An exact name, `*`, or a single leading or trailing `*`. Anything else —
/// `a*b`, `**` — matches nothing, and a processor keyed on it silently never
/// runs, which is why it is reported rather than accepted.
pub fn pattern_is_matchable(pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    match pattern.matches('*').count() {
        0 => true,
        1 => pattern.starts_with('*') || pattern.ends_with('*'),
        _ => false,
    }
}

/// The phase `method` runs at, given a processor's `methods` map.
///
/// Most specific first:
///
/// 1. An exact name (`tools/call`).
/// 2. A prefix wildcard (`tools/*`) beats a suffix wildcard (`*/list`) —
///    method names are namespaced left to right, so the namespace owner wins.
/// 3. A suffix wildcard beats the `*` catch-all.
/// 4. Within one kind the longer pattern wins, so `notifications/tools/*`
///    beats `notifications/*`.
/// 5. Remaining ties go alphabetically, so resolution is deterministic rather
///    than dependent on map iteration order.
pub fn resolve(method: &str, methods: &BTreeMap<String, Phase>) -> Phase {
    if let Some(phase) = methods.get(method) {
        return *phase;
    }

    methods
        .iter()
        .filter_map(|(pattern, phase)| specificity(pattern, method).map(|k| (k, pattern, *phase)))
        .max_by(|a, b| {
            (a.0, a.1.len())
                .cmp(&(b.0, b.1.len()))
                // Reversed, so the alphabetically *first* pattern wins the tie.
                .then_with(|| b.1.cmp(a.1))
        })
        .map(|(_, _, phase)| phase)
        .unwrap_or_default()
}

/// How specific a pattern is for this method; `None` when it does not match.
fn specificity(pattern: &str, method: &str) -> Option<u8> {
    if pattern == "*" {
        return Some(1);
    }
    if let Some(prefix) = pattern.strip_suffix('*')
        && !prefix.contains('*')
        && method.starts_with(prefix)
    {
        return Some(3);
    }
    if let Some(suffix) = pattern.strip_prefix('*')
        && !suffix.contains('*')
        && method.ends_with(suffix)
    {
        return Some(2);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn methods(pairs: &[(&str, Phase)]) -> BTreeMap<String, Phase> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn an_unmatched_method_bypasses_the_processor() {
        // `methods` is an allow-list, so silence means off rather than on.
        let m = methods(&[("tools/call", Phase::Full)]);
        assert_eq!(resolve("tools/list", &m), Phase::Off);
        assert_eq!(resolve("anything", &BTreeMap::new()), Phase::Off);
    }

    #[test]
    fn star_matches_everything() {
        let m = methods(&[("*", Phase::Request)]);
        assert_eq!(resolve("tools/call", &m), Phase::Request);
        assert_eq!(resolve("anything", &m), Phase::Request);
    }

    #[test]
    fn prefix_and_suffix_wildcards_match_their_side() {
        let m = methods(&[("tools/*", Phase::Request), ("*/list", Phase::Response)]);
        assert_eq!(resolve("tools/call", &m), Phase::Request);
        assert_eq!(resolve("prompts/list", &m), Phase::Response);
        assert_eq!(resolve("resources/read", &m), Phase::Off);
    }

    #[test]
    fn exact_beats_wildcard() {
        let m = methods(&[("tools/*", Phase::Request), ("tools/call", Phase::Full)]);
        assert_eq!(resolve("tools/call", &m), Phase::Full);
        assert_eq!(resolve("tools/list", &m), Phase::Request);
    }

    #[test]
    fn prefix_beats_suffix() {
        // `tools/list` matches both; the namespace owner wins.
        let m = methods(&[("tools/*", Phase::Request), ("*/list", Phase::Response)]);
        assert_eq!(resolve("tools/list", &m), Phase::Request);

        // And it wins on kind, not length: `/setLevel` is the longer literal.
        let m = methods(&[
            ("logging/*", Phase::Request),
            ("*/setLevel", Phase::Response),
        ]);
        assert_eq!(resolve("logging/setLevel", &m), Phase::Request);
    }

    #[test]
    fn a_wildcard_beats_the_catchall() {
        let m = methods(&[("*", Phase::Request), ("*/list", Phase::Response)]);
        assert_eq!(resolve("resources/list", &m), Phase::Response);

        let m = methods(&[("*", Phase::Request), ("tools/*", Phase::Full)]);
        assert_eq!(resolve("tools/call", &m), Phase::Full);
    }

    #[test]
    fn the_longer_prefix_wins() {
        let m = methods(&[
            ("resources/*", Phase::Request),
            ("resources/templates/*", Phase::Response),
        ]);
        assert_eq!(resolve("resources/templates/list", &m), Phase::Response);
        assert_eq!(resolve("resources/read", &m), Phase::Request);
    }

    #[test]
    fn a_tie_resolves_alphabetically_rather_than_by_iteration_order() {
        // Two prefixes of equal length both match. Without the final tie-break
        // this would depend on map ordering, and the same config would behave
        // differently on different runs.
        let m = methods(&[("aaaa/*", Phase::Request), ("bbbb/*", Phase::Response)]);
        assert_eq!(resolve("aaaa/x", &m), Phase::Request);
        assert_eq!(resolve("bbbb/x", &m), Phase::Response);
    }

    #[test]
    fn an_unmatchable_pattern_is_reported_rather_than_accepted() {
        // A processor keyed on one of these never runs, which is
        // indistinguishable from one that always passes.
        for pattern in ["a*b", "**", "", "*a*"] {
            assert!(!pattern_is_matchable(pattern), "{pattern}");
        }
        for pattern in ["tools/call", "*", "tools/*", "*/list"] {
            assert!(pattern_is_matchable(pattern), "{pattern}");
        }
    }

    #[test]
    fn phases_report_which_sides_they_run() {
        assert!(Phase::Request.runs_request() && !Phase::Request.runs_response());
        assert!(!Phase::Response.runs_request() && Phase::Response.runs_response());
        assert!(Phase::Full.runs_request() && Phase::Full.runs_response());
        assert!(!Phase::Off.runs_request() && !Phase::Off.runs_response());
    }
}
