//! Pure logic for operator-declared free-token budgets (`[[free_tiers]]`):
//! parsing settings out of config, and tracking this process's own
//! prompt+completion token usage against them per period. Reuses
//! `client_budget`'s calendar math (`period_key_at`/`roll_period_if_needed`)
//! -- same "which period are we in, has it rolled over" question, just
//! keyed by "provider/model" and counted in tokens instead of USD.
//!
//! Self-declared and never verified against the provider's actual quota,
//! same trust model as `[providers.*]`'s `zdr`/`no_training` flags: this
//! only tells you how close *you* think you are to a limit you told it
//! about, not a live reading of the provider's own systems.

use std::collections::HashMap;

use crate::client_budget::period_key_at;
use crate::config::{BudgetPeriod, FreeTierEntry};

#[derive(Debug, Clone, Copy)]
pub struct FreeTierSetting {
    pub monthly_free_tokens: u64,
    pub period: BudgetPeriod,
}

/// One "provider/model"'s tracked token usage, scoped to whichever period
/// key was current the last time it was touched -- same shape as
/// `client_budget::SpendState`, counted in tokens rather than dollars.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokenState {
    pub period_key: i64,
    pub tokens_used: u64,
}

/// "provider/model" -> `FreeTierSetting`, for every `[[free_tiers]]` entry.
/// Duplicate `model` values keep only the last entry, same "last one wins"
/// convention `[[routes]]`/`[[presets]]` already use.
pub fn settings_from_config(entries: &[FreeTierEntry]) -> HashMap<String, FreeTierSetting> {
    entries
        .iter()
        .map(|e| {
            (
                e.model.clone(),
                FreeTierSetting {
                    monthly_free_tokens: e.monthly_free_tokens,
                    period: e.period,
                },
            )
        })
        .collect()
}

/// If `key` has a configured free-tier setting, rolls its `TokenState` to
/// the current period (resetting `tokens_used` to 0 on a rollover) and
/// adds `tokens` to it. A no-op for any "provider/model" with no
/// `[[free_tiers]]` entry -- most requests never touch `usage` at all.
pub fn record_usage(
    settings: &HashMap<String, FreeTierSetting>,
    usage: &mut HashMap<String, TokenState>,
    key: &str,
    tokens: u64,
    now_unix: i64,
) {
    let Some(setting) = settings.get(key) else {
        return;
    };
    let current_key = period_key_at(setting.period, now_unix);
    let state = usage.entry(key.to_string()).or_default();
    roll_period_if_needed_tokens(state, current_key);
    state.tokens_used = state.tokens_used.saturating_add(tokens);
}

/// `roll_period_if_needed` operates on `client_budget::SpendState`
/// (dollars); this is the same rollover check for `TokenState` (tokens).
/// Kept as a tiny local wrapper rather than generalizing
/// `roll_period_if_needed` itself, since the two states don't otherwise
/// share a type.
fn roll_period_if_needed_tokens(state: &mut TokenState, current_key: i64) {
    if state.period_key != current_key {
        state.period_key = current_key;
        state.tokens_used = 0;
    }
}

/// One entry of `GET /v1/free-tiers`: a configured budget, this period's
/// tracked usage against it, and what's left. `remaining` is
/// `monthly_free_tokens.saturating_sub(tokens_used)` -- never negative,
/// even if usage has gone over budget (nothing stops a request once a
/// free-tier budget is exceeded, unlike `[[clients]].budget_usd`; this is
/// reporting-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct FreeTierStatus {
    pub monthly_free_tokens: u64,
    pub tokens_used: u64,
    pub tokens_remaining: u64,
    pub period: BudgetPeriod,
}

/// Snapshot every configured `[[free_tiers]]` entry's status as of
/// `now_unix`, rolling each one's `TokenState` to the current period first
/// (so a stale prior-period `tokens_used` never leaks into the report --
/// same rollover-on-read behavior `check_client_budget` gives spend).
/// A configured entry this process has never dispatched to yet reports
/// `tokens_used: 0`, not absent -- unlike `provider_stats`, which omits an
/// unobserved "provider/model" entirely, `[[free_tiers]]` entries are
/// operator-declared up front, so every one of them is always known.
pub fn status_snapshot(
    settings: &HashMap<String, FreeTierSetting>,
    usage: &mut HashMap<String, TokenState>,
    now_unix: i64,
) -> HashMap<String, FreeTierStatus> {
    settings
        .iter()
        .map(|(key, setting)| {
            let current_key = period_key_at(setting.period, now_unix);
            let state = usage.entry(key.clone()).or_default();
            roll_period_if_needed_tokens(state, current_key);
            (
                key.clone(),
                FreeTierStatus {
                    monthly_free_tokens: setting.monthly_free_tokens,
                    tokens_used: state.tokens_used,
                    tokens_remaining: setting
                        .monthly_free_tokens
                        .saturating_sub(state.tokens_used),
                    period: setting.period,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(model: &str, tokens: u64, period: BudgetPeriod) -> FreeTierEntry {
        FreeTierEntry {
            model: model.to_string(),
            monthly_free_tokens: tokens,
            period,
        }
    }

    // --- settings_from_config -----------------------------------------------------

    #[test]
    fn settings_from_config_builds_one_entry_per_model() {
        let settings = settings_from_config(&[
            entry(
                "mistral/mistral-large",
                1_000_000_000,
                BudgetPeriod::Monthly,
            ),
            entry(
                "groq/llama-3.3-70b-versatile",
                117_000_000,
                BudgetPeriod::Daily,
            ),
        ]);
        assert_eq!(settings.len(), 2);
        assert_eq!(
            settings["mistral/mistral-large"].monthly_free_tokens,
            1_000_000_000
        );
        assert_eq!(
            settings["groq/llama-3.3-70b-versatile"].period,
            BudgetPeriod::Daily
        );
    }

    #[test]
    fn settings_from_config_last_duplicate_wins() {
        let settings = settings_from_config(&[
            entry("groq/m1", 100, BudgetPeriod::Monthly),
            entry("groq/m1", 200, BudgetPeriod::Monthly),
        ]);
        assert_eq!(settings.len(), 1);
        assert_eq!(settings["groq/m1"].monthly_free_tokens, 200);
    }

    // --- record_usage ---------------------------------------------------------------

    #[test]
    fn record_usage_is_a_no_op_for_an_unconfigured_model() {
        let settings = HashMap::new();
        let mut usage = HashMap::new();
        record_usage(&settings, &mut usage, "groq/unconfigured", 1000, 0);
        assert!(usage.is_empty());
    }

    #[test]
    fn record_usage_accumulates_within_the_same_period() {
        let settings = settings_from_config(&[entry("groq/m1", 1000, BudgetPeriod::Total)]);
        let mut usage = HashMap::new();
        record_usage(&settings, &mut usage, "groq/m1", 100, 0);
        record_usage(&settings, &mut usage, "groq/m1", 50, 1_000_000);
        assert_eq!(usage["groq/m1"].tokens_used, 150);
    }

    #[test]
    fn record_usage_resets_on_a_period_rollover() {
        let settings = settings_from_config(&[entry("groq/m1", 1000, BudgetPeriod::Monthly)]);
        let mut usage = HashMap::new();
        record_usage(&settings, &mut usage, "groq/m1", 900, 1_704_067_200); // 2024-01-01
        record_usage(&settings, &mut usage, "groq/m1", 50, 1_706_745_600); // 2024-02-01
        assert_eq!(
            usage["groq/m1"].tokens_used, 50,
            "a new month must reset tokens_used before adding the new sample"
        );
    }

    // --- status_snapshot --------------------------------------------------------------

    #[test]
    fn status_snapshot_reports_zero_usage_for_a_never_dispatched_entry() {
        let settings = settings_from_config(&[entry("groq/m1", 1000, BudgetPeriod::Monthly)]);
        let mut usage = HashMap::new();
        let snapshot = status_snapshot(&settings, &mut usage, 0);
        assert_eq!(snapshot["groq/m1"].tokens_used, 0);
        assert_eq!(snapshot["groq/m1"].tokens_remaining, 1000);
    }

    #[test]
    fn status_snapshot_computes_remaining_from_used() {
        let settings = settings_from_config(&[entry("groq/m1", 1000, BudgetPeriod::Total)]);
        let mut usage = HashMap::new();
        record_usage(&settings, &mut usage, "groq/m1", 400, 0);
        let snapshot = status_snapshot(&settings, &mut usage, 0);
        assert_eq!(snapshot["groq/m1"].tokens_used, 400);
        assert_eq!(snapshot["groq/m1"].tokens_remaining, 600);
    }

    #[test]
    fn status_snapshot_remaining_saturates_at_zero_when_over_budget() {
        let settings = settings_from_config(&[entry("groq/m1", 1000, BudgetPeriod::Total)]);
        let mut usage = HashMap::new();
        record_usage(&settings, &mut usage, "groq/m1", 5000, 0);
        let snapshot = status_snapshot(&settings, &mut usage, 0);
        assert_eq!(snapshot["groq/m1"].tokens_used, 5000);
        assert_eq!(
            snapshot["groq/m1"].tokens_remaining, 0,
            "remaining must never go negative -- saturating_sub, not a panic or wraparound"
        );
    }

    #[test]
    fn status_snapshot_rolls_over_a_stale_period_before_reporting() {
        let settings = settings_from_config(&[entry("groq/m1", 1000, BudgetPeriod::Monthly)]);
        let mut usage = HashMap::new();
        record_usage(&settings, &mut usage, "groq/m1", 900, 1_704_067_200); // Jan
        let snapshot = status_snapshot(&settings, &mut usage, 1_706_745_600); // Feb
        assert_eq!(
            snapshot["groq/m1"].tokens_used, 0,
            "reading in a new period must not show stale prior-period usage"
        );
    }
}
