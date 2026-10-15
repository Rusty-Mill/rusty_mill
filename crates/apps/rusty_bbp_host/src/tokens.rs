//! `bbp tokens`: model tokens per turn, read from the harness output the
//! moderator already keeps (`agents/<task>/turn-N.log`). The core cannot see
//! tokens; a launcher that runs `claude -p ... --output-format stream-json`
//! prints them. A turn whose harness was killed has no final `result` line, so
//! its tokens are the sum of the assistant messages it printed (a lower bound,
//! marked `complete: false`).

use crate::moderator::log_path;
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
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

/// Dollars per million tokens for one model.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Prices {
    pub input: f64,
    pub output: f64,
    pub cache_creation: f64,
    pub cache_read: f64,
}

impl Prices {
    /// Every price finite and positive: a zero price would make a count free.
    fn check(&self) -> Result<(), String> {
        let all = [
            self.input,
            self.output,
            self.cache_creation,
            self.cache_read,
        ];
        if all.iter().all(|p| p.is_finite() && *p > 0.0) {
            Ok(())
        } else {
            Err("prices must be finite and positive".into())
        }
    }
}

impl Usage {
    /// Dollars at `p` (per million tokens).
    pub fn dollars(&self, p: &Prices) -> f64 {
        (self.input_tokens as f64 * p.input
            + self.output_tokens as f64 * p.output
            + self.cache_creation_input_tokens as f64 * p.cache_creation
            + self.cache_read_input_tokens as f64 * p.cache_read)
            / 1e6
    }
}

/// The machine-readable part of the 2b criteria, committed and frozen with
/// them (`docs/bbp/2b-criteria.json`). Unknown keys are ignored.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Criteria {
    /// The date the prices were read off the price page.
    pub prices_as_of: String,
    /// Dollars per million tokens, per model id exactly as the harness prints it.
    pub models: BTreeMap<String, Prices>,
    /// The earliest commit a run may use: the merge of the verdict-binding fix.
    /// Absent until the owner has merged it; a run refuses to start without it.
    #[rusty_serde(default)]
    pub min_commit: Option<String>,
}

/// Read the criteria file and its sha256, which the run output records.
pub fn read_criteria(file: &Path) -> Result<(Criteria, String), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let c: Criteria =
        rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    if c.prices_as_of.trim().is_empty() || c.models.is_empty() {
        return Err(format!(
            "{}: needs prices_as_of and at least one model",
            file.display()
        ));
    }
    for (m, p) in &c.models {
        p.check()
            .map_err(|e| format!("{}: {m}: {e}", file.display()))?;
    }
    Ok((c, Sha256::of(&bytes).hex()))
}

impl Criteria {
    /// Dollars for per-model usage. A model with no price is an error, never
    /// zero: an unpriced model would make a run look cheap.
    pub fn dollars(&self, by_model: &BTreeMap<String, Usage>) -> Result<f64, String> {
        by_model
            .iter()
            .map(|(m, u)| {
                self.models
                    .get(m)
                    .map(|p| u.dollars(p))
                    .ok_or_else(|| format!("model {m:?} has no price in the criteria file"))
            })
            .sum()
    }
}

/// Compare the harness's tokens (and, when both sides have it, dollars) with
/// the API key's usage, part by part. A part off by more than `tolerance` (a fraction of the API's figure), or a turn
/// with no harness output, is a mismatch.
pub fn reconcile(
    seen: &Usage,
    api: &Usage,
    cost: Option<(f64, f64)>,
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
    let mut bad: Vec<String> = parts
        .iter()
        .filter(|(_, s, a)| (*s as f64 - *a as f64).abs() > tolerance * *a as f64)
        .map(|(n, s, a)| format!("{n}: bbp {s}, api {a}"))
        .collect();
    if let Some((seen_usd, api_usd)) = cost {
        if (seen_usd - api_usd).abs() > tolerance * api_usd {
            bad.push(format!("cost: bbp ${seen_usd:.4}, api ${api_usd:.4}"));
        }
    }
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
    model: Option<String>,
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
            (
                "assistant",
                _,
                Some(Message {
                    id, usage: Some(u), ..
                }),
            ) => {
                if id.is_none_or(|id| seen.insert(id)) {
                    assistant = assistant.plus(u);
                }
            }
            _ => {}
        }
    }
    (assistant, false)
}

/// The assistant messages' usage per model, once per message id. Dollars are
/// priced from this, not from the `result` line, because a turn can use more
/// than one model (a helper agent) and the `result` line does not say which
/// tokens were whose. A message that names no model is filed under `unknown`.
pub fn by_model(stream: &str) -> BTreeMap<String, Usage> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: BTreeMap<String, Usage> = BTreeMap::new();
    for line in stream.lines() {
        let Ok(l) = rusty_serde::json::from_str::<Line>(line) else {
            continue;
        };
        let (
            true,
            Some(Message {
                id,
                model,
                usage: Some(u),
            }),
        ) = (l.kind == "assistant", l.message)
        else {
            continue;
        };
        if id.is_none_or(|id| seen.insert(id)) {
            let m = model.unwrap_or_else(|| "unknown".into());
            let e = out.entry(m).or_default();
            *e = e.plus(u);
        }
    }
    out
}

/// One turn's tokens.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TurnTokens {
    pub turn: u64,
    pub role: String,
    /// The turn was revoked: its work was thrown away.
    pub revoked: bool,
    /// The harness printed a final `result`; false is a lower bound.
    pub complete: bool,
    pub usage: Usage,
    /// Dollars, when a criteria file priced it.
    pub usd: Option<f64>,
    /// The assistant messages' tokens per model (what `usd` is priced from).
    pub models: BTreeMap<String, Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskTokens {
    pub turns: Vec<TurnTokens>,
    /// The four counts summed over every turn.
    pub sum: Usage,
    /// Dollars over every turn, when a criteria file priced them.
    pub usd: Option<f64>,
    /// Every turn, revoked ones included.
    pub total: u64,
    pub revoked: u64,
    /// Turns that were granted but left no harness output (no launcher, or it
    /// never started): their cost is unknown, not zero.
    pub without_output: u64,
}

/// Per-turn tokens of `task`, from its log and the harness output under `dir`.
pub fn of(
    dir: &Path,
    task: &TaskId,
    events: &[Event],
    criteria: Option<&Criteria>,
) -> Result<TaskTokens, String> {
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
        usd: None,
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
        let models = by_model(&text);
        let usd = criteria
            .map(|c| {
                c.dollars(&models)
                    .map_err(|e| format!("turn {}: {e}", turn.0))
            })
            .transpose()?;
        out.usd = match (out.usd, usd) {
            (Some(a), Some(b)) => Some(a + b),
            (None, Some(b)) => Some(b),
            (a, None) => a,
        };
        out.turns.push(TurnTokens {
            turn: turn.0,
            role: role.0.clone(),
            revoked,
            complete,
            usage,
            usd,
            models,
        });
    }
    if criteria.is_some() && out.usd.is_none() {
        out.usd = Some(0.0);
    }
    Ok(out)
}

/// What `bbp tokens` prints: the tokens, and the criteria file they were
/// priced by.
#[derive(Serialize)]
struct Printed {
    tokens: TaskTokens,
    criteria_sha256: Option<String>,
    prices_as_of: Option<String>,
}

/// The tolerance of the pilot reconciliation (a fraction of the API's figure).
pub const RECONCILE_TOLERANCE: f64 = 0.02;

/// The API key's usage for the pilot, as copied from its usage report.
#[derive(Deserialize)]
struct ApiUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    #[rusty_serde(default)]
    cost_usd: Option<f64>,
}

/// [`of`] as one line of JSON, priced by `criteria` when given. With
/// `api_usage` it also reconciles, and a mismatch is an error: the run stops.
pub fn json(
    dir: &Path,
    task: &TaskId,
    criteria: Option<&Path>,
    api_usage: Option<&Path>,
) -> Result<String, String> {
    let d = crate::open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    let loaded = criteria.map(read_criteria).transpose()?;
    let t = of(dir, task, &events, loaded.as_ref().map(|(c, _)| c))?;
    if let Some(f) = api_usage {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let a: ApiUsage =
            rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))?;
        let api = Usage {
            input_tokens: a.input_tokens,
            output_tokens: a.output_tokens,
            cache_creation_input_tokens: a.cache_creation_input_tokens,
            cache_read_input_tokens: a.cache_read_input_tokens,
        };
        let cost = a.cost_usd.zip(t.usd);
        // The API reports dollars only if the file has them; both sides or neither.
        let cost = cost.map(|(api_usd, seen)| (seen, api_usd));
        reconcile(&t.sum, &api, cost, t.without_output, RECONCILE_TOLERANCE)?;
    }
    rusty_serde::json::to_string(&Printed {
        tokens: t,
        criteria_sha256: loaded.as_ref().map(|(_, h)| h.clone()),
        prices_as_of: loaded.map(|(c, _)| c.prices_as_of),
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
        let got = of(&dir, &task, &events, None).unwrap();
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

    const OPUS: Prices = Prices {
        input: 4.0,
        output: 20.0,
        cache_creation: 5.0,
        cache_read: 0.2,
    };
    const SONNET: Prices = Prices {
        input: 2.0,
        output: 10.0,
        cache_creation: 2.5,
        cache_read: 0.2,
    };

    fn criteria() -> Criteria {
        Criteria {
            prices_as_of: "2026-10-06".into(),
            models: [
                ("claude-opus-5-5".to_owned(), OPUS),
                ("claude-sonnet-5-5".to_owned(), SONNET),
            ]
            .into(),
            min_commit: None,
        }
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
    fn dollars_are_price_per_million_times_tokens() {
        // 1M of each part at Opus prices: 4 + 20 + 5 + 0.2.
        let u = usage(1_000_000, 1_000_000, 1_000_000, 1_000_000);
        assert!((u.dollars(&OPUS) - 29.2).abs() < 1e-9);
        assert_eq!(u.total(), 4_000_000, "the plain sum is unchanged");
    }

    #[test]
    fn a_mixed_roster_is_priced_per_model_and_an_unpriced_model_is_an_error() {
        let mut m = BTreeMap::new();
        m.insert("claude-opus-5-5".to_owned(), usage(1_000_000, 0, 0, 0));
        m.insert("claude-sonnet-5-5".to_owned(), usage(1_000_000, 0, 0, 0));
        assert!((criteria().dollars(&m).unwrap() - 6.0).abs() < 1e-9);
        m.insert("claude-haiku-5-5".to_owned(), usage(1, 0, 0, 0));
        let err = criteria().dollars(&m).unwrap_err();
        assert!(
            err.contains("claude-haiku-5-5") && err.contains("no price"),
            "{err}"
        );
    }

    #[test]
    fn by_model_prices_each_message_by_its_own_model_once_per_id() {
        let line = |id: &str, model: &str, i: u64| {
            format!(
                r#"{{"type":"assistant","message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":{i},"output_tokens":0}}}}}}"#
            )
        };
        let s = [
            line("m1", "claude-opus-5-5", 10),
            line("m1", "claude-opus-5-5", 10),
            line("m2", "claude-sonnet-5-5", 7),
        ]
        .join("\n");
        let got = by_model(&s);
        assert_eq!(got["claude-opus-5-5"].input_tokens, 10);
        assert_eq!(got["claude-sonnet-5-5"].input_tokens, 7);
    }

    #[test]
    fn a_bad_price_or_missing_date_is_refused_and_the_hash_is_recorded() {
        let f = std::env::temp_dir().join(format!("crit-{}.json", std::process::id()));
        let body = r#"{"status":"proposal","prices_as_of":"2026-10-06","models":{"claude-opus-5-5":{"input":4,"output":20,"cache_creation":5,"cache_read":0.2}}}"#;
        std::fs::write(&f, body).unwrap();
        let (c, h) = read_criteria(&f).unwrap();
        assert_eq!(c.models["claude-opus-5-5"], OPUS);
        assert_eq!(c.min_commit, None);
        assert_eq!(h, Sha256::of(body.as_bytes()).hex());
        for bad in [body.replace("\"cache_read\":0.2", "\"cache_read\":0"), body.replace("2026-10-06", " "), body.replace("\"claude-opus-5-5\":{\"input\":4,\"output\":20,\"cache_creation\":5,\"cache_read\":0.2}", "")] {
            std::fs::write(&f, &bad).unwrap();
            assert!(read_criteria(&f).is_err(), "{bad}");
        }
        std::fs::remove_file(f).unwrap();
    }

    #[test]
    fn reconciliation_allows_two_percent_per_part_and_stops_beyond() {
        let api = usage(1000, 500, 200, 10_000);
        assert!(reconcile(&usage(1000, 500, 200, 10_000), &api, None, 0, 0.02).is_ok());
        assert!(
            reconcile(&usage(980, 510, 204, 9_800), &api, None, 0, 0.02).is_ok(),
            "on the edge"
        );
        let err = reconcile(&usage(1000, 500, 200, 9_700), &api, None, 0, 0.02).unwrap_err();
        assert!(err.contains("cache_read: bbp 9700, api 10000"), "{err}");
    }

    #[test]
    fn dollars_are_reconciled_too_when_the_api_reports_them() {
        let u = usage(1, 1, 1, 1);
        assert!(reconcile(&u, &u, Some((10.0, 10.1)), 0, 0.02).is_ok());
        let err = reconcile(&u, &u, Some((10.0, 10.5)), 0, 0.02).unwrap_err();
        assert!(err.contains("cost:"), "{err}");
    }

    #[test]
    fn a_turn_with_no_output_stops_the_reconciliation_even_if_the_rest_matches() {
        let u = usage(1, 1, 1, 1);
        assert!(reconcile(&u, &u, None, 1, 0.02)
            .unwrap_err()
            .contains("no harness output"));
    }

    #[test]
    fn a_part_the_api_never_reports_must_be_zero_in_the_capture() {
        let api = usage(10, 10, 0, 0);
        assert!(reconcile(&usage(10, 10, 0, 0), &api, None, 0, 0.02).is_ok());
        assert!(reconcile(&usage(10, 10, 1, 0), &api, None, 0, 0.02).is_err());
    }

    #[test]
    fn the_committed_criteria_file_parses() {
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/bbp/2b-criteria.json");
        let (c, h) = read_criteria(&f).unwrap();
        assert!(c.models.contains_key("claude-opus-5-5"));
        assert_eq!(h.len(), 64);
    }
}
