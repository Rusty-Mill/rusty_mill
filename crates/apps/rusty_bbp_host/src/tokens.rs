//! `bbp tokens`: model tokens per turn, read from the harness output the
//! moderator already keeps (`agents/<task>/turn-N.log`). The core cannot see
//! tokens; a launcher that runs `claude -p ... --output-format stream-json`
//! prints them. A turn whose harness was killed has no final `result` line, so
//! its tokens are the sum of the assistant messages it printed (a lower bound,
//! marked `complete: false`).

use crate::moderator::log_path;
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// Tokens as the harness reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    #[rusty_serde(default)]
    pub input_tokens: u64,
    #[rusty_serde(default)]
    pub output_tokens: u64,
    #[rusty_serde(default)]
    pub cache_creation_input_tokens: u64,
    #[rusty_serde(default)]
    pub cache_read_input_tokens: u64,
}

impl Usage {
    /// Everything the harness consumed. The parts stay in the output so the
    /// owner can weigh cache reads differently.
    pub fn total(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_creation_input_tokens
            + self.cache_read_input_tokens
    }

    fn plus(self, o: Usage) -> Usage {
        Usage {
            input_tokens: self.input_tokens + o.input_tokens,
            output_tokens: self.output_tokens + o.output_tokens,
            cache_creation_input_tokens: self.cache_creation_input_tokens
                + o.cache_creation_input_tokens,
            cache_read_input_tokens: self.cache_read_input_tokens + o.cache_read_input_tokens,
        }
    }
}

/// Cost weights in input-token equivalents. Per model, so the criteria file
/// names the model the harness runs on.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Weights {
    pub input: f64,
    pub output: f64,
    pub cache_creation: f64,
    pub cache_read: f64,
}

impl Weights {
    /// Every weight finite and positive: a zero or negative weight would
    /// make a count free or negative.
    fn check(&self) -> Result<(), String> {
        let all = [
            self.input,
            self.output,
            self.cache_creation,
            self.cache_read,
        ];
        if all.iter().all(|w| w.is_finite() && *w > 0.0) {
            Ok(())
        } else {
            Err("weights must be finite and positive".into())
        }
    }
}

impl Usage {
    /// Cost in input-token equivalents.
    pub fn weighted(&self, w: &Weights) -> f64 {
        self.input_tokens as f64 * w.input
            + self.output_tokens as f64 * w.output
            + self.cache_creation_input_tokens as f64 * w.cache_creation
            + self.cache_read_input_tokens as f64 * w.cache_read
    }
}

/// The machine-readable part of the 2b criteria, committed and frozen with
/// them (`docs/bbp/2b-criteria.json`). Unknown keys are ignored.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Criteria {
    pub model: String,
    pub weights: Weights,
}

/// Read the criteria file and its sha256, which the run output records.
pub fn read_criteria(file: &Path) -> Result<(Criteria, String), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let c: Criteria =
        rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    c.weights
        .check()
        .map_err(|e| format!("{}: {e}", file.display()))?;
    Ok((c, Sha256::of(&bytes).hex()))
}

/// Compare the harness's tokens with the API key's usage, part by part. A part
/// off by more than `tolerance` (a fraction of the API's figure), or a turn
/// with no harness output, is a mismatch.
pub fn reconcile(
    seen: &Usage,
    api: &Usage,
    without_output: u64,
    tolerance: f64,
) -> Result<(), String> {
    if without_output > 0 {
        return Err(format!("{without_output} turn(s) left no harness output"));
    }
    let parts = [
        ("input", seen.input_tokens, api.input_tokens),
        ("output", seen.output_tokens, api.output_tokens),
        (
            "cache_creation",
            seen.cache_creation_input_tokens,
            api.cache_creation_input_tokens,
        ),
        (
            "cache_read",
            seen.cache_read_input_tokens,
            api.cache_read_input_tokens,
        ),
    ];
    let bad: Vec<String> = parts
        .iter()
        .filter(|(_, s, a)| (*s as f64 - *a as f64).abs() > tolerance * *a as f64)
        .map(|(n, s, a)| format!("{n}: bbp {s}, api {a}"))
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "token capture does not match the API usage: {}",
            bad.join("; ")
        ))
    }
}

#[derive(Deserialize)]
struct Message {
    #[rusty_serde(default)]
    id: Option<String>,
    #[rusty_serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Line {
    #[rusty_serde(rename = "type")]
    kind: String,
    #[rusty_serde(default)]
    usage: Option<Usage>,
    #[rusty_serde(default)]
    message: Option<Message>,
}

/// The tokens one harness printed and whether its final `result` line was
/// there. Lines that are not JSON, or carry no usage, are skipped. Assistant
/// messages are counted once per message id (a message is printed once per
/// content block).
pub fn parse(stream: &str) -> (Usage, bool) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut assistant = Usage::default();
    for line in stream.lines() {
        let Ok(l) = rusty_serde::json::from_str::<Line>(line) else {
            continue;
        };
        match (l.kind.as_str(), l.usage, l.message) {
            ("result", Some(u), _) => return (u, true),
            ("assistant", _, Some(Message { id, usage: Some(u) })) => {
                if id.is_none_or(|id| seen.insert(id)) {
                    assistant = assistant.plus(u);
                }
            }
            _ => {}
        }
    }
    (assistant, false)
}

/// One turn's tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TurnTokens {
    pub turn: u64,
    pub role: String,
    /// The turn was revoked: its work was thrown away.
    pub revoked: bool,
    /// The harness printed a final `result`; false is a lower bound.
    pub complete: bool,
    pub usage: Usage,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskTokens {
    pub turns: Vec<TurnTokens>,
    /// The four counts summed over every turn.
    pub sum: Usage,
    /// Cost-weighted total, when a criteria file gave weights.
    pub weighted: Option<f64>,
    /// Every turn, revoked ones included.
    pub total: u64,
    pub revoked: u64,
    /// Turns that were granted but left no harness output (no launcher, or it
    /// never started): their cost is unknown, not zero.
    pub without_output: u64,
}

/// Per-turn tokens of `task`, from its log and the harness output under `dir`.
pub fn of(dir: &Path, task: &TaskId, events: &[Event]) -> TaskTokens {
    let revoked: HashSet<TurnId> = events
        .iter()
        .filter_map(|e| match e {
            Event::TurnEnded {
                turn,
                cause: TurnEnd::Revoked,
            } => Some(*turn),
            _ => None,
        })
        .collect();
    let mut out = TaskTokens {
        turns: Vec::new(),
        sum: Usage::default(),
        weighted: None,
        total: 0,
        revoked: 0,
        without_output: 0,
    };
    for e in events {
        let Event::TurnGranted { role, turn, .. } = e else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(log_path(dir, task, *turn)) else {
            out.without_output += 1;
            continue;
        };
        let (usage, complete) = parse(&text);
        let revoked = revoked.contains(turn);
        out.total += usage.total();
        out.sum = out.sum.plus(usage);
        if revoked {
            out.revoked += usage.total();
        }
        out.turns.push(TurnTokens {
            turn: turn.0,
            role: role.0.clone(),
            revoked,
            complete,
            usage,
        });
    }
    out
}

/// What `bbp tokens` prints: the tokens, and the criteria file they were
/// weighted by.
#[derive(Serialize)]
struct Printed {
    tokens: TaskTokens,
    criteria_sha256: Option<String>,
}

/// The tolerance of the pilot reconciliation (a fraction of the API's figure).
pub const RECONCILE_TOLERANCE: f64 = 0.02;

/// [`of`] as one line of JSON, weighted by `criteria` when given. With
/// `api_usage` (a file of the four counts from the key's usage report) it also
/// reconciles, and a mismatch is an error: the run stops.
pub fn json(
    dir: &Path,
    task: &TaskId,
    criteria: Option<&Path>,
    api_usage: Option<&Path>,
) -> Result<String, String> {
    let d = crate::open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    let mut t = of(dir, task, &events);
    let mut sha = None;
    if let Some(f) = criteria {
        let (c, h) = read_criteria(f)?;
        t.weighted = Some(t.turns.iter().map(|x| x.usage.weighted(&c.weights)).sum());
        sha = Some(h);
    }
    if let Some(f) = api_usage {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let api: Usage =
            rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))?;
        reconcile(&t.sum, &api, t.without_output, RECONCILE_TOLERANCE)?;
    }
    rusty_serde::json::to_string(&Printed {
        tokens: t,
        criteria_sha256: sha,
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const RESULT: &str = r#"{"type":"result","subtype":"success","usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"server_tool_use":{"web_search_requests":0}},"total_cost_usd":0.1}"#;

    fn assistant(id: &str, input: u64, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}","usage":{{"input_tokens":{input},"output_tokens":{output}}}}}}}"#
        )
    }

    #[test]
    fn the_result_line_is_authoritative() {
        let s = format!("{}\nnot json\n{RESULT}\n", assistant("m1", 1, 1));
        let (u, complete) = parse(&s);
        assert!(complete);
        assert_eq!((u.input_tokens, u.output_tokens, u.total()), (10, 5, 1115));
    }

    #[test]
    fn a_killed_harness_is_summed_from_assistant_messages_once_per_id() {
        // The same message printed twice (two content blocks) counts once.
        let s = [
            assistant("m1", 3, 2),
            assistant("m1", 3, 2),
            assistant("m2", 4, 1),
        ]
        .join("\n");
        let (u, complete) = parse(&s);
        assert!(!complete);
        assert_eq!((u.input_tokens, u.output_tokens), (7, 3));
    }

    #[test]
    fn output_with_no_usage_is_zero_and_incomplete() {
        assert_eq!(
            parse("hello\n{\"type\":\"system\"}\n"),
            (Usage::default(), false)
        );
    }

    #[test]
    fn a_task_sums_every_turn_and_marks_revoked_and_missing_output() {
        let dir = std::env::temp_dir().join(format!("bbp-tokens-{}", std::process::id()));
        let task = TaskId("T".into());
        let t = |n| TurnId(n);
        let granted = |n, role: &str| Event::TurnGranted {
            role: RoleId(role.into()),
            turn: t(n),
            kind: TurnKind::Default,
            deadline: Time(0),
            return_to: None,
            slice: None,
        };
        for (n, body) in [(1, RESULT.to_owned()), (2, assistant("m", 7, 3))] {
            let p = log_path(&dir, &task, t(n));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        let events = [
            granted(1, "planner"),
            granted(2, "coder2"),
            granted(3, "tester"), // no output
            Event::TurnEnded {
                turn: t(2),
                cause: TurnEnd::Revoked,
            },
        ];
        let got = of(&dir, &task, &events);
        assert_eq!(got.total, 1115 + 10);
        assert_eq!(got.revoked, 10);
        assert_eq!(got.without_output, 1);
        assert_eq!(
            got.turns
                .iter()
                .map(|x| (x.role.as_str(), x.revoked, x.complete))
                .collect::<Vec<_>>(),
            [("planner", false, true), ("coder2", true, false)]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    const W: Weights = Weights {
        input: 1.0,
        output: 5.0,
        cache_creation: 1.25,
        cache_read: 0.05,
    };

    #[test]
    fn weighted_cost_is_the_sum_of_weighted_parts() {
        let u = Usage {
            input_tokens: 10,
            output_tokens: 5,
            cache_creation_input_tokens: 100,
            cache_read_input_tokens: 1000,
        };
        // 10 + 25 + 125 + 50
        assert_eq!(u.weighted(&W), 210.0);
        assert_eq!(u.total(), 1115, "the plain sum is unchanged");
    }

    #[test]
    fn a_non_positive_or_non_finite_weight_is_refused() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let w = Weights { output: bad, ..W };
            assert!(w.check().is_err(), "{bad}");
        }
        assert!(W.check().is_ok());
    }

    #[test]
    fn the_criteria_file_is_read_with_its_hash_and_unknown_keys_ignored() {
        let f = std::env::temp_dir().join(format!("crit-{}.json", std::process::id()));
        let body = r#"{"status":"proposal","model":"claude-opus-5-5","weights":{"input":1,"output":5,"cache_creation":1.25,"cache_read":0.05}}"#;
        std::fs::write(&f, body).unwrap();
        let (c, h) = read_criteria(&f).unwrap();
        assert_eq!((c.model.as_str(), c.weights), ("claude-opus-5-5", W));
        assert_eq!(h, Sha256::of(body.as_bytes()).hex());
        std::fs::write(&f, body.replace("1.25", "0")).unwrap();
        assert!(read_criteria(&f).is_err());
        std::fs::remove_file(f).unwrap();
    }

    fn usage(i: u64, o: u64, w: u64, r: u64) -> Usage {
        Usage {
            input_tokens: i,
            output_tokens: o,
            cache_creation_input_tokens: w,
            cache_read_input_tokens: r,
        }
    }

    #[test]
    fn reconciliation_allows_two_percent_per_part_and_stops_beyond() {
        let api = usage(1000, 500, 200, 10_000);
        assert!(reconcile(&usage(1000, 500, 200, 10_000), &api, 0, 0.02).is_ok());
        assert!(
            reconcile(&usage(980, 510, 204, 9_800), &api, 0, 0.02).is_ok(),
            "on the edge"
        );
        let err = reconcile(&usage(1000, 500, 200, 9_700), &api, 0, 0.02).unwrap_err();
        assert!(err.contains("cache_read: bbp 9700, api 10000"), "{err}");
    }

    #[test]
    fn a_turn_with_no_output_stops_the_reconciliation_even_if_the_rest_matches() {
        let u = usage(1, 1, 1, 1);
        assert!(reconcile(&u, &u, 1, 0.02)
            .unwrap_err()
            .contains("no harness output"));
    }

    #[test]
    fn a_part_the_api_never_reports_must_be_zero_in_the_capture() {
        let api = usage(10, 10, 0, 0);
        assert!(reconcile(&usage(10, 10, 0, 0), &api, 0, 0.02).is_ok());
        assert!(reconcile(&usage(10, 10, 1, 0), &api, 0, 0.02).is_err());
    }

    #[test]
    fn the_committed_criteria_file_parses() {
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/bbp/2b-criteria.json");
        let (c, h) = read_criteria(&f).unwrap();
        assert_eq!(c.weights.input, 1.0);
        assert_eq!(h.len(), 64);
    }
}
