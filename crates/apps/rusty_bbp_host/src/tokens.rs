//! `bbp tokens`: what each turn cost, read from the harness output the
//! moderator already keeps (`agents/<task>/turn-N.log`).
//!
//! The harness is `claude -p ... --output-format stream-json --verbose`
//! (Claude Code 2.1.300; real captures are in `tests/fixtures/stream-json/`).
//! The `result` line is the only complete account of a turn: its `modelUsage`
//! is per model and includes subagents, its `total_cost_usd` is the harness's
//! own figure. The per-message `usage` on `assistant` lines is not: its
//! `output_tokens` is a placeholder from `message_start`, and subagent messages
//! are not printed at all. So dollars are priced only from a valid final
//! `result`; a turn without one (a killed harness, a crash, an interrupt, which
//! prints a result with zeroed usage) has an unknown cost, never a smaller one.
//! `modelUsage` does not split cache writes by lifetime (the `usage` object
//! does, but excludes subagents), so cache writes are priced at the 1-hour
//! rate and the moderator pins both cache TTL variables to 1 hour; a run that
//! shows 5-minute writes, or whose priced total disagrees with the harness's
//! `total_cost_usd`, is refused.

use crate::moderator::log_path;
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// The cache lifetime the moderator pins for the main agent and for subagents.
pub const PINNED_CACHE_TTL: &str = "1h";
/// The environment variables that set them.
pub const CACHE_TTL_ENV: [&str; 2] = [
    "CLAUDE_CODE_PROMPT_CACHE_TTL",
    "CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL",
];
/// How far the priced total may be from the harness's `total_cost_usd`.
pub const COST_CROSS_CHECK: f64 = 0.02;

/// The API's split of cache writes by lifetime, when the usage carries it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheCreation {
    #[rusty_serde(default)]
    pub ephemeral_5m_input_tokens: u64,
    #[rusty_serde(default)]
    pub ephemeral_1h_input_tokens: u64,
}

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
    /// The breakdown of `cache_creation_input_tokens` by lifetime, where a
    /// source prints it (an API usage report does; `modelUsage` does not).
    #[rusty_serde(default)]
    pub cache_creation: CacheCreation,
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
            cache_creation: CacheCreation {
                ephemeral_5m_input_tokens: self.cache_creation.ephemeral_5m_input_tokens
                    + o.cache_creation.ephemeral_5m_input_tokens,
                ephemeral_1h_input_tokens: self.cache_creation.ephemeral_1h_input_tokens
                    + o.cache_creation.ephemeral_1h_input_tokens,
            },
        }
    }

    /// Cache writes split into (5-minute, everything else). Only tokens the
    /// usage proves were 5-minute writes are priced at the 5-minute rate; the
    /// rest, including any a missing or short breakdown leaves unexplained, is
    /// priced at the 1-hour rate, so cost is never understated. (This holds
    /// while a model's 1-hour price is not below its 5-minute price, which
    /// [`Prices::check`] enforces.)
    pub fn cache_writes(&self) -> (u64, u64) {
        let five = self
            .cache_creation
            .ephemeral_5m_input_tokens
            .min(self.cache_creation_input_tokens);
        (five, self.cache_creation_input_tokens - five)
    }
}

/// Dollars per million tokens for one model.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Prices {
    pub input: f64,
    pub output: f64,
    /// Cache write, 5-minute lifetime.
    pub cache_creation_5m: f64,
    /// Cache write, 1-hour lifetime (2x input).
    pub cache_creation_1h: f64,
    pub cache_read: f64,
}

impl Prices {
    /// Every price finite and positive, and a 1-hour write no cheaper than a
    /// 5-minute one: a zero price would make a count free, and the rule that a
    /// missing split can only raise the cost needs the order.
    fn check(&self) -> Result<(), String> {
        let all = [
            self.input,
            self.output,
            self.cache_creation_5m,
            self.cache_creation_1h,
            self.cache_read,
        ];
        if !all.iter().all(|p| p.is_finite() && *p > 0.0) {
            return Err("prices must be finite and positive".into());
        }
        if self.cache_creation_1h < self.cache_creation_5m {
            return Err("cache_creation_1h must not be below cache_creation_5m".into());
        }
        Ok(())
    }
}

impl Usage {
    /// Dollars at `p` (per million tokens).
    pub fn dollars(&self, p: &Prices) -> f64 {
        let (five, hour) = self.cache_writes();
        (self.input_tokens as f64 * p.input
            + self.output_tokens as f64 * p.output
            + five as f64 * p.cache_creation_5m
            + hour as f64 * p.cache_creation_1h
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
/// the API key's usage, part by part. A part off by more than `tolerance` (a
/// fraction of the API's figure), a non-finite dollar figure, or a turn whose
/// cost is unknown is a mismatch.
pub fn reconcile(
    seen: &Usage,
    api: &Usage,
    cost: Option<(f64, f64)>,
    unknown_turns: u64,
    tolerance: f64,
) -> Result<(), String> {
    if unknown_turns > 0 {
        return Err(format!(
            "{unknown_turns} turn(s) have no usable harness result, so they cannot be reconciled"
        ));
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
    // The harness's modelUsage does not split cache writes, so there is no
    // split on this side to compare with the report's; the dollars below carry
    // the lifetime (a 5-minute write is priced lower).
    if let Some((seen_usd, api_usd)) = cost {
        if !seen_usd.is_finite() || !api_usd.is_finite() {
            return Err(format!(
                "cost is not a finite number (bbp {seen_usd}, api {api_usd})"
            ));
        }
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

// ---------------------------------------------------------------- parsing

/// A `modelUsage` entry of a `result` line.
#[derive(Deserialize)]
struct ModelUsageLine {
    #[rusty_serde(rename = "inputTokens", default)]
    input: u64,
    #[rusty_serde(rename = "outputTokens", default)]
    output: u64,
    #[rusty_serde(rename = "cacheReadInputTokens", default)]
    cache_read: u64,
    #[rusty_serde(rename = "cacheCreationInputTokens", default)]
    cache_creation: u64,
}

impl ModelUsageLine {
    fn usage(&self) -> Usage {
        Usage {
            input_tokens: self.input,
            output_tokens: self.output,
            cache_creation_input_tokens: self.cache_creation,
            cache_read_input_tokens: self.cache_read,
            cache_creation: CacheCreation::default(),
        }
    }
}

/// The `usage` object of a `result` line, where the harness reports how the
/// requests were served.
#[derive(Deserialize)]
struct ResultUsage {
    #[rusty_serde(default)]
    speed: Option<String>,
    #[rusty_serde(default)]
    service_tier: Option<String>,
    #[rusty_serde(default)]
    inference_geo: Option<String>,
    #[rusty_serde(default)]
    cache_creation: CacheCreation,
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
    message: Option<Message>,
    #[rusty_serde(default)]
    is_error: bool,
    #[rusty_serde(default)]
    terminal_reason: Option<String>,
    #[rusty_serde(default)]
    total_cost_usd: Option<f64>,
    #[rusty_serde(default)]
    fast_mode_state: Option<String>,
    #[rusty_serde(default)]
    usage: Option<ResultUsage>,
    #[rusty_serde(rename = "modelUsage", default)]
    model_usage: BTreeMap<String, ModelUsageLine>,
}

/// A final `result` line.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultInfo {
    pub is_error: bool,
    pub terminal_reason: Option<String>,
    pub total_cost_usd: Option<f64>,
    /// Per model, subagents included.
    pub models: BTreeMap<String, Usage>,
    speed: Option<String>,
    service_tier: Option<String>,
    inference_geo: Option<String>,
    fast_mode_state: Option<String>,
    /// 5-minute cache writes the main agent's `usage` reports.
    five_minute_writes: u64,
}

/// The last `result` line of a harness's output. A log can hold several (a
/// background task starts another turn); the last is the account of the whole
/// session. Lines that are not JSON are skipped.
pub fn last_result(stream: &str) -> Option<ResultInfo> {
    stream
        .lines()
        .rev()
        .filter_map(|l| rusty_serde::json::from_str::<Line>(l).ok())
        .find(|l| l.kind == "result")
        .map(|l| {
            let u = l.usage;
            ResultInfo {
                is_error: l.is_error,
                terminal_reason: l.terminal_reason,
                total_cost_usd: l.total_cost_usd,
                models: l
                    .model_usage
                    .iter()
                    .map(|(m, u)| (m.clone(), u.usage()))
                    .collect(),
                fast_mode_state: l.fast_mode_state,
                speed: u.as_ref().and_then(|u| u.speed.clone()),
                service_tier: u.as_ref().and_then(|u| u.service_tier.clone()),
                inference_geo: u.as_ref().and_then(|u| u.inference_geo.clone()),
                five_minute_writes: u
                    .as_ref()
                    .map_or(0, |u| u.cache_creation.ephemeral_5m_input_tokens),
            }
        })
}

/// The assistant messages' usage per model, once per message id. A lower bound
/// only: each message's `output_tokens` is a placeholder from `message_start`
/// and subagent messages are not printed. Used for the live cap and for what a
/// turn without a result is known to have spent. A message that names no model
/// is filed under `unknown`.
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
        // Claude Code files its own error messages (an unrecognized model, an
        // API failure) under the model name `<synthetic>`, with zero usage. It
        // is not a model anyone can be billed for.
        if model.as_deref() == Some("<synthetic>") {
            continue;
        }
        if id.is_none_or(|id| seen.insert(id)) {
            let m = model.unwrap_or_else(|| "unknown".into());
            let e = out.entry(m).or_default();
            *e = e.plus(u);
        }
    }
    out
}

/// What a turn's output says it cost.
#[derive(Debug, Clone, PartialEq)]
pub enum Cost {
    /// A valid final result, priced and cross-checked.
    Known {
        usd: f64,
        usage: Usage,
        models: BTreeMap<String, Usage>,
    },
    /// No usable result: why.
    Unknown(String),
}

/// Price a harness's output. `Err` means the output is refusable (a model with
/// no price, fast mode, a priority tier, regional inference, 5-minute cache
/// writes, or a total that disagrees with the harness's own `total_cost_usd`);
/// `Ok(Unknown)` means there is nothing to price.
pub fn price_turn(stream: &str, c: &Criteria) -> Result<Cost, String> {
    let Some(r) = last_result(stream) else {
        return Ok(Cost::Unknown("no result line".into()));
    };
    if r.is_error || r.terminal_reason.as_deref() != Some("completed") {
        return Ok(Cost::Unknown(format!(
            "result is not a completed turn (is_error {}, terminal_reason {:?})",
            r.is_error, r.terminal_reason
        )));
    }
    let usage = r.models.values().fold(Usage::default(), |a, u| a.plus(*u));
    if r.models.is_empty() || usage.total() == 0 {
        return Ok(Cost::Unknown("the result reports no usage".into()));
    }
    for (what, got, ok) in [
        ("speed", &r.speed, "standard"),
        ("service_tier", &r.service_tier, "standard"),
        ("fast_mode_state", &r.fast_mode_state, "off"),
    ] {
        if let Some(v) = got {
            if v != ok {
                return Err(format!(
                    "{what} is {v:?}, which the price file does not price"
                ));
            }
        }
    }
    if let Some(g) = &r.inference_geo {
        if !g.is_empty() && g != "global" {
            return Err(format!(
                "inference_geo is {g:?}, which the price file does not price"
            ));
        }
    }
    if r.five_minute_writes > 0 {
        return Err(format!(
            "{} cache tokens were written for 5 minutes; {} must be pinned to {}",
            r.five_minute_writes,
            CACHE_TTL_ENV.join(" and "),
            PINNED_CACHE_TTL
        ));
    }
    let usd = c.dollars(&r.models)?;
    let total = r
        .total_cost_usd
        .filter(|t| t.is_finite())
        .ok_or("the result has no total_cost_usd to cross-check")?;
    if (usd - total).abs() > COST_CROSS_CHECK * total + 1e-9 {
        return Err(format!(
            "priced ${usd:.6} but the harness reports total_cost_usd ${total:.6}: the price file or the cache TTL does not match what ran"
        ));
    }
    Ok(Cost::Known {
        usd,
        usage,
        models: r.models,
    })
}

/// One turn's tokens.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TurnTokens {
    pub turn: u64,
    pub role: String,
    /// The turn was revoked: its work was thrown away.
    pub revoked: bool,
    /// The turn has a valid final result; false means its cost is unknown and
    /// `usage` is only what the printed assistant messages show.
    pub complete: bool,
    /// Why the cost is unknown, when it is.
    pub unknown: Option<String>,
    pub usage: Usage,
    /// Dollars, when a criteria file priced a complete turn.
    pub usd: Option<f64>,
    /// A floor on the dollars: the complete turn's cost, or what its printed
    /// assistant messages add up to. For the live cap, never for criteria.
    pub usd_floor: Option<f64>,
    /// The tokens per model (from the result when complete).
    pub models: BTreeMap<String, Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskTokens {
    pub turns: Vec<TurnTokens>,
    /// The four counts summed over every turn.
    pub sum: Usage,
    /// Dollars over every turn: known only if every turn is.
    pub usd: Option<f64>,
    /// A floor on the dollars over every turn.
    pub usd_floor: Option<f64>,
    /// Every turn, revoked ones included.
    pub total: u64,
    pub revoked: u64,
    /// Turns that were granted but left no harness output (no launcher, or it
    /// never started): their cost is unknown, not zero.
    pub without_output: u64,
    /// Turns with output but no usable result.
    pub unknown_turns: u64,
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
        usd_floor: None,
        total: 0,
        revoked: 0,
        without_output: 0,
        unknown_turns: 0,
    };
    let (mut usd, mut floor) = (0.0, 0.0);
    let mut all_known = true;
    for e in events {
        let Event::TurnGranted { role, turn, .. } = e else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(log_path(dir, task, *turn)) else {
            out.without_output += 1;
            all_known = false;
            continue;
        };
        let lower = by_model(&text);
        let cost = criteria
            .map(|c| price_turn(&text, c).map_err(|e| format!("turn {}: {e}", turn.0)))
            .transpose()?;
        let (complete, unknown, turn_usd, models) = match cost {
            Some(Cost::Known { usd, models, .. }) => (true, None, Some(usd), models),
            Some(Cost::Unknown(why)) => (false, Some(why), None, lower.clone()),
            None => {
                // No prices given: the tokens only, from the result if there is
                // one, else the printed messages.
                match last_result(&text).filter(|r| !r.is_error && !r.models.is_empty()) {
                    Some(r) => (true, None, None, r.models),
                    None => (
                        false,
                        Some("no usable result line".into()),
                        None,
                        lower.clone(),
                    ),
                }
            }
        };
        let usage = models.values().fold(Usage::default(), |a, u| a.plus(*u));
        let turn_floor = match (criteria, turn_usd) {
            (Some(_), Some(d)) => Some(d),
            (Some(c), None) => Some(
                c.dollars(&lower)
                    .map_err(|e| format!("turn {}: {e}", turn.0))?,
            ),
            _ => None,
        };
        let is_revoked = revoked.contains(turn);
        out.total += usage.total();
        out.sum = out.sum.plus(usage);
        if is_revoked {
            out.revoked += usage.total();
        }
        if !complete {
            out.unknown_turns += 1;
            all_known = false;
        }
        usd += turn_usd.unwrap_or(0.0);
        floor += turn_floor.unwrap_or(0.0);
        out.turns.push(TurnTokens {
            turn: turn.0,
            role: role.0.clone(),
            revoked: is_revoked,
            complete,
            unknown,
            usage,
            usd: turn_usd,
            usd_floor: turn_floor,
            models,
        });
    }
    if criteria.is_some() {
        out.usd_floor = Some(floor);
        out.usd = all_known.then_some(usd);
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
    /// The report's split of cache writes by lifetime, when it has one.
    #[rusty_serde(default)]
    cache_creation: CacheCreation,
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
            cache_creation: a.cache_creation,
        };
        // The report's dollars need ours: with no prices or an unknown turn
        // the dollar check cannot run, and silently skipping it would pass.
        let cost = match a.cost_usd {
            None => None,
            Some(api_usd) => Some((
                t.usd.ok_or(
                    "the API report has a cost, but this capture has no priced total (pass --criteria, and every turn needs a result)",
                )?,
                api_usd,
            )),
        };
        reconcile(
            &t.sum,
            &api,
            cost,
            t.without_output + t.unknown_turns,
            RECONCILE_TOLERANCE,
        )?;
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

    const OPUS: Prices = Prices {
        input: 4.0,
        output: 20.0,
        cache_creation_5m: 5.0,
        cache_creation_1h: 8.0,
        cache_read: 0.2,
    };
    const SONNET: Prices = Prices {
        input: 2.0,
        output: 10.0,
        cache_creation_5m: 2.5,
        cache_creation_1h: 4.0,
        cache_read: 0.1,
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
            cache_creation: CacheCreation::default(),
        }
    }

    fn written(total: u64, five: u64, hour: u64) -> Usage {
        Usage {
            cache_creation_input_tokens: total,
            cache_creation: CacheCreation {
                ephemeral_5m_input_tokens: five,
                ephemeral_1h_input_tokens: hour,
            },
            ..Usage::default()
        }
    }

    #[test]
    fn dollars_are_price_per_million_times_tokens() {
        // No split is printed, so cache writes are priced at the 1-hour rate:
        // 4 + 20 + 8 + 0.2 for 1M of each part.
        let u = usage(1_000_000, 1_000_000, 1_000_000, 1_000_000);
        assert!((u.dollars(&OPUS) - 32.2).abs() < 1e-9);
        assert_eq!(u.total(), 4_000_000, "the plain sum is unchanged");
    }

    #[test]
    fn a_printed_split_prices_five_minute_and_one_hour_writes_separately() {
        let u = written(2_000_000, 1_000_000, 1_000_000);
        assert!((u.dollars(&OPUS) - 13.0).abs() < 1e-9);
        assert!((u.dollars(&SONNET) - 6.5).abs() < 1e-9);
    }

    #[test]
    fn writes_the_split_does_not_explain_are_priced_at_the_one_hour_rate() {
        let none = written(1_000_000, 0, 0);
        assert_eq!(none.cache_writes(), (0, 1_000_000));
        let short = written(1_000_000, 300_000, 0);
        assert_eq!(short.cache_writes(), (300_000, 700_000));
        assert_eq!(
            written(100, 500, 0).cache_writes(),
            (100, 0),
            "capped at the total"
        );
        assert!(short.dollars(&OPUS) < none.dollars(&OPUS));
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

    fn assistant(id: &str, model: Option<&str>, i: u64) -> String {
        let m = model.map_or(String::new(), |m| format!(r#""model":"{m}","#));
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}",{m}"usage":{{"input_tokens":{i},"output_tokens":0}}}}}}"#
        )
    }

    #[test]
    fn by_model_files_each_message_under_its_model_once_per_id_and_keeps_unnamed_ones() {
        let s = [
            assistant("m1", Some("claude-opus-5-5"), 10),
            assistant("m1", Some("claude-opus-5-5"), 10),
            assistant("m2", Some("claude-sonnet-5-5"), 7),
            assistant("m3", None, 5),
        ]
        .join("\n");
        let got = by_model(&s);
        assert_eq!(got["claude-opus-5-5"].input_tokens, 10);
        assert_eq!(got["claude-sonnet-5-5"].input_tokens, 7);
        // A message with no model is kept (and later refused as unpriced), not dropped.
        assert_eq!(got["unknown"].input_tokens, 5);
        assert!(criteria().dollars(&got).is_err());
    }

    #[test]
    fn the_last_result_line_is_chosen_and_non_json_lines_are_skipped() {
        let line = |cost: f64| {
            format!(
                r#"{{"type":"result","is_error":false,"terminal_reason":"completed","total_cost_usd":{cost},"modelUsage":{{"m":{{"inputTokens":1,"outputTokens":1,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}}}}}}"#
            )
        };
        let s = format!("garbage\n{}\nnot json\n{}\n", line(1.0), line(2.0));
        assert_eq!(last_result(&s).unwrap().total_cost_usd, Some(2.0));
        assert_eq!(last_result("nothing\n"), None);
    }

    #[test]
    fn a_bad_price_or_missing_date_is_refused_and_the_hash_is_recorded() {
        let f = std::env::temp_dir().join(format!("crit-{}.json", std::process::id()));
        let body = r#"{"status":"proposal","prices_as_of":"2026-10-06","models":{"claude-opus-5-5":{"input":4,"output":20,"cache_creation_5m":5,"cache_creation_1h":8,"cache_read":0.2}}}"#;
        std::fs::write(&f, body).unwrap();
        let (c, h) = read_criteria(&f).unwrap();
        assert_eq!(c.models["claude-opus-5-5"], OPUS);
        assert_eq!(c.min_commit, None);
        assert_eq!(h, Sha256::of(body.as_bytes()).hex());
        for bad in [
            body.replace("\"cache_read\":0.2", "\"cache_read\":0"),
            body.replace("2026-10-06", " "),
            body.replace("\"cache_creation_1h\":8", "\"cache_creation_1h\":0"),
            // A 1-hour write cheaper than a 5-minute write breaks the rule
            // that a missing split can only raise the price.
            body.replace("\"cache_creation_1h\":8", "\"cache_creation_1h\":4"),
            body.replace("\"claude-opus-5-5\":{\"input\":4,\"output\":20,\"cache_creation_5m\":5,\"cache_creation_1h\":8,\"cache_read\":0.2}", ""),
        ] {
            std::fs::write(&f, &bad).unwrap();
            assert!(read_criteria(&f).is_err(), "{bad}");
        }
        // Equal 5-minute and 1-hour prices are allowed.
        std::fs::write(
            &f,
            body.replace("\"cache_creation_1h\":8", "\"cache_creation_1h\":5"),
        )
        .unwrap();
        assert!(read_criteria(&f).is_ok());
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
        // Each part, on its own, is compared.
        for (name, bad) in [
            ("input", usage(900, 500, 200, 10_000)),
            ("output", usage(1000, 400, 200, 10_000)),
            ("cache_creation", usage(1000, 500, 100, 10_000)),
        ] {
            assert!(
                reconcile(&bad, &api, None, 0, 0.02)
                    .unwrap_err()
                    .contains(name),
                "{name}"
            );
        }
    }

    #[test]
    fn dollars_are_reconciled_too_and_only_when_finite() {
        let u = usage(1, 1, 1, 1);
        assert!(reconcile(&u, &u, Some((10.0, 10.1)), 0, 0.02).is_ok());
        assert!(reconcile(&u, &u, Some((10.0, 10.5)), 0, 0.02)
            .unwrap_err()
            .contains("cost:"));
        for (a, b) in [(f64::NAN, 1.0), (1.0, f64::NAN), (1.0, f64::INFINITY)] {
            assert!(reconcile(&u, &u, Some((a, b)), 0, 0.02).is_err(), "{a} {b}");
        }
    }

    #[test]
    fn a_turn_with_no_usable_result_stops_the_reconciliation_even_if_the_rest_matches() {
        let u = usage(1, 1, 1, 1);
        assert!(reconcile(&u, &u, None, 1, 0.02)
            .unwrap_err()
            .contains("no usable"));
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

    fn log_dir(tag: &str) -> (std::path::PathBuf, TaskId) {
        let d = std::env::temp_dir().join(format!("tok-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        (d, TaskId("T".into()))
    }

    fn granted(n: u64, role: &str) -> Event {
        Event::TurnGranted {
            role: RoleId(role.into()),
            turn: TurnId(n),
            kind: TurnKind::Default,
            deadline: Time(0),
            return_to: None,
            slice: None,
        }
    }

    #[test]
    fn a_task_with_no_turns_has_a_zero_priced_total_and_a_revoked_turn_is_counted() {
        let (dir, task) = log_dir("none");
        let none = of(&dir, &task, &[], Some(&criteria())).unwrap();
        assert_eq!(
            (none.usd, none.unknown_turns, none.without_output),
            (Some(0.0), 0, 0)
        );
        let p = log_path(&dir, &task, TurnId(1));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let line = r#"{"type":"result","is_error":false,"terminal_reason":"completed","total_cost_usd":0.000008,"modelUsage":{"claude-sonnet-5-5":{"inputTokens":4,"outputTokens":0,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}}}"#;
        std::fs::write(&p, line).unwrap();
        let events = [
            granted(1, "coder2"),
            Event::TurnEnded {
                turn: TurnId(1),
                cause: TurnEnd::Revoked,
            },
        ];
        let t = of(&dir, &task, &events, Some(&criteria())).unwrap();
        assert!(
            t.turns[0].revoked && t.revoked == 4,
            "a revoked turn's tokens are counted"
        );
        assert!((t.usd.unwrap() - 0.000008).abs() < 1e-12, "and its dollars");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_turn_with_no_log_file_is_unknown_and_makes_the_task_unknown() {
        let (dir, task) = log_dir("nolog");
        let t = of(&dir, &task, &[granted(1, "planner")], Some(&criteria())).unwrap();
        assert_eq!((t.without_output, t.usd), (1, None));
    }
}
