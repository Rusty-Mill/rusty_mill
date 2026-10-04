//! Budgets and the hard-stop cost meter (ADR-0005 §5, invariant 2).
//!
//! Cost is metered in model tokens plus wall-clock time, with GPU time when
//! the host reports it. It is never dollars: a local model costs about
//! nothing, so a dollar budget would stop constraining the search.
//!
//! [`CostMeter`] reads no clock. The runtime reports elapsed time and token
//! usage; the meter only does the accounting, so the stop rule is identical
//! for every candidate and testable without sleeping.

use core::time::Duration;

use rusty_err::Error;

use crate::error::CoreError;

/// The per-task resource limit a candidate runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    tokens: u64,
    wall: Duration,
    gpu: Option<Duration>,
}

impl Budget {
    /// A budget of `tokens` model tokens, `wall` wall-clock time and, when
    /// set, `gpu` GPU time.
    ///
    /// # Errors
    /// [`CoreError::EmptyBudget`] if `tokens`, `wall` or a set `gpu` is zero:
    /// such a budget would refuse every operation.
    pub fn new(tokens: u64, wall: Duration, gpu: Option<Duration>) -> Result<Self, CoreError> {
        if tokens == 0 {
            return Err(CoreError::EmptyBudget("token"));
        }
        if wall.is_zero() {
            return Err(CoreError::EmptyBudget("wall-clock"));
        }
        if gpu.is_some_and(|limit| limit.is_zero()) {
            return Err(CoreError::EmptyBudget("GPU"));
        }
        Ok(Self { tokens, wall, gpu })
    }

    /// The token limit (prompt plus completion).
    #[must_use]
    pub const fn tokens(&self) -> u64 {
        self.tokens
    }

    /// The wall-clock limit.
    #[must_use]
    pub const fn wall(&self) -> Duration {
        self.wall
    }

    /// The GPU-time limit, if GPU time is budgeted.
    #[must_use]
    pub const fn gpu(&self) -> Option<Duration> {
        self.gpu
    }
}

/// Resources actually consumed. Plain data: lineage records it as is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CostUsage {
    /// Prompt tokens charged by the model endpoint.
    pub prompt_tokens: u64,
    /// Completion tokens charged by the model endpoint.
    pub completion_tokens: u64,
    /// Wall-clock time elapsed.
    pub wall: Duration,
    /// GPU time, when the host can measure it.
    pub gpu: Option<Duration>,
}

impl CostUsage {
    /// Total tokens, saturating rather than wrapping.
    #[must_use]
    pub const fn tokens(&self) -> u64 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }

    /// The sum of two usages, for totals across tasks or rounds.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        let gpu = match (self.gpu, other.gpu) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or_default().saturating_add(b.unwrap_or_default())),
        };
        Self {
            prompt_tokens: self.prompt_tokens.saturating_add(other.prompt_tokens),
            completion_tokens: self
                .completion_tokens
                .saturating_add(other.completion_tokens),
            wall: self.wall.saturating_add(other.wall),
            gpu,
        }
    }
}

/// Which limit stopped a run. Usage at or past a limit is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BudgetExhausted {
    /// The token limit was reached.
    #[error("token budget exhausted: {used} of {limit} tokens")]
    Tokens {
        /// Tokens used.
        used: u64,
        /// The limit.
        limit: u64,
    },
    /// The wall-clock limit was reached.
    #[error("wall-clock budget exhausted: {used:?} of {limit:?}")]
    Wall {
        /// Time elapsed.
        used: Duration,
        /// The limit.
        limit: Duration,
    },
    /// The GPU-time limit was reached.
    #[error("GPU budget exhausted: {used:?} of {limit:?}")]
    Gpu {
        /// GPU time used.
        used: Duration,
        /// The limit.
        limit: Duration,
    },
}

/// Accounts usage against a [`Budget`] and enforces the hard stop.
///
/// The protocol is admit-then-record: call [`CostMeter::admit`] before each
/// operation and record its usage afterwards. Recording never fails, since
/// the cost has already been incurred and must reach lineage; only the next
/// admission is refused. The one in-flight operation can therefore overshoot;
/// callers bound that by asking for at most [`CostMeter::remaining_tokens`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostMeter {
    budget: Budget,
    used: CostUsage,
}

impl CostMeter {
    /// A meter with nothing used yet.
    #[must_use]
    pub fn new(budget: Budget) -> Self {
        Self {
            budget,
            used: CostUsage::default(),
        }
    }

    /// Whether another operation may start.
    ///
    /// # Errors
    /// The first limit (tokens, then wall-clock, then GPU) that usage has
    /// reached or passed.
    pub fn admit(&self) -> Result<(), BudgetExhausted> {
        let used_tokens = self.used.tokens();
        if used_tokens >= self.budget.tokens {
            return Err(BudgetExhausted::Tokens {
                used: used_tokens,
                limit: self.budget.tokens,
            });
        }
        if self.used.wall >= self.budget.wall {
            return Err(BudgetExhausted::Wall {
                used: self.used.wall,
                limit: self.budget.wall,
            });
        }
        match (self.used.gpu, self.budget.gpu) {
            (Some(used), Some(limit)) if used >= limit => Err(BudgetExhausted::Gpu { used, limit }),
            _ => Ok(()),
        }
    }

    /// Charges the tokens one model call consumed.
    pub fn record_tokens(&mut self, prompt: u64, completion: u64) {
        self.used.prompt_tokens = self.used.prompt_tokens.saturating_add(prompt);
        self.used.completion_tokens = self.used.completion_tokens.saturating_add(completion);
    }

    /// Reports total wall-clock time elapsed since the run started.
    ///
    /// Elapsed time only moves forward, so a smaller value than one already
    /// seen is ignored rather than refunding time.
    pub fn observe_wall(&mut self, elapsed: Duration) {
        self.used.wall = self.used.wall.max(elapsed);
    }

    /// Charges GPU time.
    pub fn record_gpu(&mut self, spent: Duration) {
        let total = self.used.gpu.unwrap_or_default().saturating_add(spent);
        self.used.gpu = Some(total);
    }

    /// Tokens left before the token limit; zero once it is reached.
    #[must_use]
    pub fn remaining_tokens(&self) -> u64 {
        self.budget.tokens.saturating_sub(self.used.tokens())
    }

    /// Usage so far.
    #[must_use]
    pub const fn usage(&self) -> &CostUsage {
        &self.used
    }

    /// The budget being enforced.
    #[must_use]
    pub const fn budget(&self) -> &Budget {
        &self.budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn meter(tokens: u64, wall_secs: u64, gpu_secs: Option<u64>) -> CostMeter {
        let gpu = gpu_secs.map(Duration::from_secs);
        CostMeter::new(Budget::new(tokens, Duration::from_secs(wall_secs), gpu).expect("valid"))
    }

    #[test]
    fn budget_rejects_zero_limits() {
        assert_eq!(
            Budget::new(0, SECOND, None),
            Err(CoreError::EmptyBudget("token"))
        );
        assert_eq!(
            Budget::new(1, Duration::ZERO, None),
            Err(CoreError::EmptyBudget("wall-clock"))
        );
        assert_eq!(
            Budget::new(1, SECOND, Some(Duration::ZERO)),
            Err(CoreError::EmptyBudget("GPU"))
        );
    }

    #[test]
    fn fresh_meter_admits() {
        assert_eq!(meter(10, 10, Some(10)).admit(), Ok(()));
    }

    #[test]
    fn token_limit_is_a_hard_stop_at_the_boundary() {
        let mut m = meter(100, 60, None);
        m.record_tokens(60, 39);
        assert_eq!(m.admit(), Ok(()), "one token left");
        assert_eq!(m.remaining_tokens(), 1);
        m.record_tokens(0, 1);
        assert_eq!(
            m.admit(),
            Err(BudgetExhausted::Tokens {
                used: 100,
                limit: 100
            })
        );
        assert_eq!(m.remaining_tokens(), 0);
    }

    #[test]
    fn overshoot_is_recorded_not_lost() {
        let mut m = meter(100, 60, None);
        m.record_tokens(80, 70);
        assert_eq!(m.usage().tokens(), 150);
        assert_eq!(
            m.admit(),
            Err(BudgetExhausted::Tokens {
                used: 150,
                limit: 100
            })
        );
        assert_eq!(m.remaining_tokens(), 0);
    }

    #[test]
    fn wall_limit_is_a_hard_stop_at_the_boundary() {
        let mut m = meter(100, 10, None);
        m.observe_wall(Duration::from_millis(9_999));
        assert_eq!(m.admit(), Ok(()));
        m.observe_wall(Duration::from_secs(10));
        assert_eq!(
            m.admit(),
            Err(BudgetExhausted::Wall {
                used: Duration::from_secs(10),
                limit: Duration::from_secs(10)
            })
        );
    }

    #[test]
    fn wall_time_never_moves_backwards() {
        let mut m = meter(100, 10, None);
        m.observe_wall(Duration::from_secs(8));
        m.observe_wall(Duration::from_secs(3));
        assert_eq!(m.usage().wall, Duration::from_secs(8));
    }

    #[test]
    fn gpu_limit_applies_only_when_budgeted() {
        let mut unbudgeted = meter(100, 10, None);
        unbudgeted.record_gpu(Duration::from_secs(1_000));
        assert_eq!(unbudgeted.admit(), Ok(()));
        assert_eq!(unbudgeted.usage().gpu, Some(Duration::from_secs(1_000)));

        let mut budgeted = meter(100, 10, Some(5));
        budgeted.record_gpu(Duration::from_secs(2));
        budgeted.record_gpu(Duration::from_secs(2));
        assert_eq!(budgeted.admit(), Ok(()));
        budgeted.record_gpu(SECOND);
        assert!(matches!(budgeted.admit(), Err(BudgetExhausted::Gpu { .. })));
    }

    #[test]
    fn tokens_are_reported_before_wall_when_both_are_exhausted() {
        let mut m = meter(1, 1, None);
        m.record_tokens(1, 0);
        m.observe_wall(SECOND);
        assert!(matches!(m.admit(), Err(BudgetExhausted::Tokens { .. })));
    }

    #[test]
    fn identical_usage_gives_identical_decisions() {
        let run = || {
            let mut m = meter(50, 5, None);
            let mut admitted = 0;
            while m.admit().is_ok() {
                m.record_tokens(3, 4);
                m.observe_wall(Duration::from_millis(100 * admitted));
                admitted += 1;
            }
            (admitted, *m.usage())
        };
        assert_eq!(run(), run());
        assert_eq!(run().0, 8, "7 tokens per call: the 8th call crosses 50");
    }

    #[test]
    fn token_accounting_saturates() {
        let mut m = meter(u64::MAX, 1, None);
        m.record_tokens(u64::MAX, u64::MAX);
        assert_eq!(m.usage().tokens(), u64::MAX);
        assert!(m.admit().is_err());
    }

    #[test]
    fn usage_plus_sums_fields() {
        let a = CostUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            wall: SECOND,
            gpu: None,
        };
        let b = CostUsage {
            prompt_tokens: 3,
            completion_tokens: 4,
            wall: SECOND,
            gpu: Some(SECOND),
        };
        let sum = a.plus(b);
        assert_eq!(sum.tokens(), 10);
        assert_eq!(sum.wall, Duration::from_secs(2));
        assert_eq!(sum.gpu, Some(SECOND));
        assert_eq!(a.plus(a).gpu, None);
    }
}
