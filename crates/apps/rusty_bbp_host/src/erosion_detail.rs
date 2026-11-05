//! Erosion detail for an inbox escalation (card #704 slice 3, D43, D45, D91).
//!
//! The `erosion` bin prints one JSON line into its section of the run log
//! (`=== erosion : …` up to `--- erosion: …`). An escalated claim shows that
//! line from one report: the report that put `erosion` in its `failing`
//! set, or else the latest report on its current candidate that ran the
//! profile, so `unmeasured` code is visible even while report-only. The
//! parse is host-side and tolerant: a log that can't be read or has no
//! version-1 line gives no detail, and the inbox still answers.

use rusty_bbp::claims::ClaimView;
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize, Value};

/// The profile name the erosion gate runs under (D82).
const PROFILE: &str = crate::profiles::EROSION_PROFILE;

/// What an escalation shows about erosion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ErosionDetail {
    /// The report the detail comes from.
    pub report: ArtId,
    /// Whether `erosion` is in the claim's failing set (the fail-block).
    pub failing: bool,
    /// The bin's own error line, when it could not measure.
    pub error: Option<String>,
    /// The measurement, when the bin printed one.
    pub line: Option<ErosionLine>,
}

/// The bin's measurement line (`rusty_erosion` `render`, version 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErosionLine {
    pub base: String,
    pub mode: ErosionMode,
    /// `None` while report-only: there are no limits to be within.
    pub within_limits: Option<bool>,
    pub concentration_bp: Share,
    pub duplication_bp: Share,
    pub total_cc: i64,
    pub loc: i64,
    pub functions: i64,
    pub worst_affected: Vec<Affected>,
    pub unmeasured: Vec<String>,
    pub parse_errors: Vec<String>,
}

/// `"report-only"`, or the enforced limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[rusty_serde(untagged)]
pub enum ErosionMode {
    ReportOnly(String),
    Limits {
        max_concentration_bp: i64,
        max_duplication_bp: i64,
    },
}

/// One metric in basis points, at the base and on the candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    pub base: u32,
    pub candidate: u32,
    pub delta: i32,
}

/// A function whose complexity rose the most.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Affected {
    /// `path::function`. Named `key` in the line; renamed here because the
    /// derive can't take a field called `key`.
    #[rusty_serde(rename = "key")]
    pub function: String,
    pub base_cc: u32,
    pub cc: u32,
}

/// The erosion detail for `claim`, reading log blobs with `blob`.
pub fn for_claim(
    st: &TaskState,
    claim: &ClaimView,
    blob: impl Fn(&BlobRef) -> Option<Vec<u8>>,
) -> Option<ErosionDetail> {
    let failing = claim
        .failing
        .iter()
        .find(|f| f.profile == PROFILE)
        .and_then(|f| f.report);
    let report = failing.or_else(|| latest_run(st, claim.current?))?;
    let log = match &st.artifacts.get(&report)?.payload {
        ArtifactPayload::TestReport(r) => r.log,
        _ => return None,
    };
    let bytes = blob(&st.artifacts.get(&log)?.blob)?;
    let (error, line) = parse(section(&String::from_utf8_lossy(&bytes))?)?;
    Some(ErosionDetail {
        report,
        failing: failing.is_some(),
        error,
        line,
    })
}

/// The latest report on `candidate` that ran the erosion profile.
fn latest_run(st: &TaskState, candidate: ArtId) -> Option<ArtId> {
    st.artifacts.values().rev().find_map(|a| match &a.payload {
        ArtifactPayload::TestReport(r)
            if r.candidate == candidate && r.profiles.iter().any(|p| p.name == PROFILE) =>
        {
            Some(a.id)
        }
        _ => None,
    })
}

/// The bin's JSON line in the erosion section of a run log.
pub fn section(log: &str) -> Option<&str> {
    let open = format!("=== {PROFILE} : ");
    let close = format!("--- {PROFILE}: ");
    log.lines()
        .skip_while(|l| !l.starts_with(&open))
        .skip(1)
        .take_while(|l| !l.starts_with(&close))
        .find(|l| l.starts_with("{\"erosion\":"))
}

/// Parse a version-1 line into the bin's error or its measurement. Any other
/// version, or a line that doesn't parse, is `None`.
#[allow(clippy::type_complexity)]
pub fn parse(line: &str) -> Option<(Option<String>, Option<ErosionLine>)> {
    let v: Value = rusty_serde::json::from_str(line).ok()?;
    if v.get("erosion")?.as_u64()? != 1 {
        return None;
    }
    if let Some(e) = v.get("error") {
        return Some((Some(e.as_str()?.to_owned()), None));
    }
    let m: ErosionLine = rusty_serde::json::from_str(line).ok()?;
    Some((None, Some(m)))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A line as `rusty_erosion`'s `render` prints it, enforcing.
    pub const ENFORCING: &str = r#"{"erosion":1,"base":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","mode":{"max_concentration_bp":50,"max_duplication_bp":50},"within_limits":false,"concentration_bp":{"base":0,"candidate":0,"delta":0},"duplication_bp":{"base":0,"candidate":4800,"delta":4800},"total_cc":2,"loc":11,"functions":1,"worst_affected":[{"key":"src/b.rs::grand","base_cc":0,"cc":2}],"unmeasured":["tool.py"],"parse_errors":[]}"#;

    #[test]
    fn a_measurement_line_parses_in_both_modes() {
        let (err, line) = parse(ENFORCING).expect("parse");
        assert_eq!(err, None);
        let line = line.expect("line");
        assert_eq!(
            line.mode,
            ErosionMode::Limits {
                max_concentration_bp: 50,
                max_duplication_bp: 50
            }
        );
        assert_eq!(line.within_limits, Some(false));
        assert_eq!(line.duplication_bp.delta, 4800);
        assert_eq!(line.worst_affected[0].function, "src/b.rs::grand");
        assert_eq!(line.unmeasured, ["tool.py"]);

        let report_only = ENFORCING
            .replace(
                r#"{"max_concentration_bp":50,"max_duplication_bp":50}"#,
                r#""report-only""#,
            )
            .replace(r#""within_limits":false"#, r#""within_limits":null"#);
        let line = parse(&report_only).expect("parse").1.expect("line");
        assert_eq!(line.mode, ErosionMode::ReportOnly("report-only".into()));
        assert_eq!(line.within_limits, None);
    }

    #[test]
    fn an_error_line_is_the_error_and_other_versions_are_nothing() {
        let (err, line) = parse(r#"{"erosion":1,"error":"git: bad base"}"#).expect("parse");
        assert_eq!(err.as_deref(), Some("git: bad base"));
        assert_eq!(line, None);
        assert_eq!(
            parse(&ENFORCING.replace(r#""erosion":1"#, r#""erosion":2"#)),
            None
        );
        assert_eq!(parse("{\"erosion\":1,\"base\":"), None);
        assert_eq!(parse(r#"{"erosion":1,"base":"x"}"#), None);
    }

    #[test]
    fn the_section_is_found_between_its_markers_only() {
        let log = format!(
            "=== test : sh ./test.sh\n{{\"erosion\":1,\"error\":\"not me\"}}\n--- test: exit 0 in 1ms\n\
             === erosion : /usr/local/bin/erosion check --base a --report-only\n\
             note\n{ENFORCING}\n--- erosion: exit 1 in 2ms\n"
        );
        assert_eq!(section(&log), Some(ENFORCING));
        assert_eq!(section("=== test : x\n--- test: exit 0\n"), None);
        let empty = "=== erosion : x\n--- erosion: exit 0\n{\"erosion\":1}\n";
        assert_eq!(section(empty), None, "a line after the section is not it");
    }
}
