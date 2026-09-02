//! The criteria judge (PRD 05 §CriteriaJudge). A semantic check: does the reply
//! satisfy the active task's success criteria? The aisdk call is injected by the
//! caller (the post-turn join in `app`); this module owns the prompt + the
//! parse. Degradation is explicit — a call/parse failure is `judge_unavailable`,
//! **never** a silent pass (it bars `AutonomousVerifiedSuccess`).

use serde::Deserialize;

/// The judge's verdict, folded into the report by
/// [`VerificationReport::with_judge`](crate::VerificationReport::with_judge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeResult {
    /// Whether all criteria were judged met.
    pub passed: bool,
    /// Whether the judge actually ran (parsed a verdict). `false` ⇒
    /// `judge_unavailable` (never a pass).
    pub judge_ran: bool,
    /// Human-readable detail (per-criterion reasons, or the failure cause).
    pub detail: String,
}

impl JudgeResult {
    /// The judge could not be reached or its output was unusable.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            passed: false,
            judge_ran: false,
            detail: reason.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct JudgeJson {
    verdict: String,
    #[serde(default)]
    criteria: Vec<JudgedCriterion>,
}

#[derive(Debug, Deserialize)]
struct JudgedCriterion {
    #[serde(default)]
    criterion: String,
    met: bool,
    #[serde(default)]
    reason: String,
}

/// Build the judge prompt for `reply` against `goal` + `criteria` (PRD 05).
pub fn judge_prompt(reply: &str, goal: &str, criteria: &[String]) -> String {
    let mut p = String::from("You are a success-criteria judge for an AI assistant.\n\n");
    p.push_str(&format!("Task goal: {goal}\n\n"));
    p.push_str("Success criteria — all must be satisfied for the task to be complete:\n");
    for (i, c) in criteria.iter().enumerate() {
        p.push_str(&format!("{}. {c}\n", i + 1));
    }
    p.push_str(&format!("\nAssistant reply:\n{reply}\n\n"));
    p.push_str(
        "A criterion is met only if the reply clearly and explicitly addresses it — do not \
         infer what is not stated.\n\nReturn ONLY valid JSON:\n\
         {\"verdict\": \"pass\"|\"fail\", \"criteria\": [{\"criterion\": \"…\", \"met\": bool, \
         \"reason\": \"…\"}]}\n",
    );
    p
}

/// Parse the judge's JSON reply. Unparseable output ⇒ `judge_unavailable`
/// (graceful degradation; never a silent pass).
pub fn parse_judge(emit_json: &str) -> JudgeResult {
    let Ok(j) = serde_json::from_str::<JudgeJson>(emit_json) else {
        return JudgeResult::unavailable("judge unavailable: unparseable verdict");
    };
    let passed = j.verdict.eq_ignore_ascii_case("pass");
    let detail = if passed {
        "all criteria met".to_string()
    } else {
        let unmet: Vec<String> = j
            .criteria
            .iter()
            .filter(|c| !c.met)
            .map(|c| format!("{}: {}", c.criterion, c.reason))
            .collect();
        if unmet.is_empty() {
            "criteria not met".to_string()
        } else {
            unmet.join("; ")
        }
    };
    JudgeResult {
        passed,
        judge_ran: true,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_verdict_parses_as_passed() {
        let jr = parse_judge(
            r#"{"verdict":"pass","criteria":[{"criterion":"x","met":true,"reason":"ok"}]}"#,
        );
        assert!(jr.passed && jr.judge_ran);
    }

    #[test]
    fn fail_verdict_collects_unmet_reasons() {
        let jr = parse_judge(
            r#"{"verdict":"fail","criteria":[{"criterion":"adds test","met":false,"reason":"no test"}]}"#,
        );
        assert!(!jr.passed && jr.judge_ran);
        assert!(jr.detail.contains("adds test: no test"));
    }

    #[test]
    fn garbage_is_judge_unavailable_not_a_pass() {
        let jr = parse_judge("not json");
        assert!(!jr.passed && !jr.judge_ran);
    }

    #[test]
    fn prompt_lists_criteria() {
        let p = judge_prompt("reply", "the goal", &["a".into(), "b".into()]);
        assert!(p.contains("Task goal: the goal"));
        assert!(p.contains("1. a"));
        assert!(p.contains("2. b"));
    }
}
