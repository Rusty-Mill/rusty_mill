//! The money meter (#598 4.2, 4.3): integer micro-dollars, reserved before
//! each call so parallel jobs cannot jointly overshoot a ceiling. Enforced
//! from the first paid call (owner answer Q4).

use std::collections::BTreeMap;
use std::fmt;

use crate::port::Usage;

/// One million, the per-token price denominator.
const MILLION: u64 = 1_000_000;

/// Micro-dollars per million tokens. Subscription CLIs and local models are
/// priced at 0 and still metered in tokens and calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Price {
    pub input: u64,
    pub output: u64,
    pub cached_input: u64,
}

impl Price {
    /// True when every rate is zero.
    pub fn is_free(&self) -> bool {
        self.input == 0 && self.output == 0 && self.cached_input == 0
    }
}

/// `tokens × rate / 1_000_000`, rounded up, saturating.
fn charge(tokens: u64, rate: u64) -> u64 {
    let product = u128::from(tokens) * u128::from(rate);
    u64::try_from(product.div_ceil(u128::from(MILLION))).unwrap_or(u64::MAX)
}

/// What a finished call cost. Cached input is billed at its own rate.
pub fn cost(u: &Usage, p: &Price) -> u64 {
    let fresh = u.input_tokens.saturating_sub(u.cached_input_tokens);
    charge(fresh, p.input)
        .saturating_add(charge(u.cached_input_tokens, p.cached_input))
        .saturating_add(charge(u.output_tokens, p.output))
}

/// Input tokens a provider may add around a prompt: roles, message
/// separators, a system preamble.
pub const FRAMING_TOKENS: u64 = 1_024;

/// The most a call can cost: every prompt byte as its own input token,
/// plus [`FRAMING_TOKENS`], each at the dearer of the fresh and cached
/// input rates, plus every output token allowed, plus one micro-dollar for
/// [`cost`] rounding the fresh and cached buckets up separately. Byte-level
/// tokenizers never emit more tokens than bytes, so this bounds [`cost`]
/// for any split of the input into fresh and cached tokens; it is not the
/// estimate context fitting uses.
pub fn worst_case(prompt_bytes: u64, max_output_tokens: u32, p: &Price) -> u64 {
    let input = prompt_bytes.saturating_add(FRAMING_TOKENS);
    let rate = p.input.max(p.cached_input);
    charge(input, rate)
        .saturating_add(charge(u64::from(max_output_tokens), p.output))
        .saturating_add(1)
}

/// The ceilings, from Factory config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ceilings {
    /// All tasks under one Factory config, in micro-dollars.
    pub experiment: u64,
    /// One BBP task, in micro-dollars.
    pub task: u64,
    /// One BBP task, every provider.
    pub task_calls: u32,
}

/// Why a call may not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The experiment ceiling: no new paid call; free calls go on.
    Experiment,
    /// The task's spend ceiling: pause the task.
    TaskSpend,
    /// The task's call ceiling: pause the task.
    TaskCalls,
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Experiment => "experiment spend ceiling reached",
            Self::TaskSpend => "task spend ceiling reached",
            Self::TaskCalls => "task call ceiling reached",
        })
    }
}

/// A reservation's handle.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticket(u64);

/// Spend for one task, beside the experiment total it shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meter {
    ceilings: Ceilings,
    /// Settled spend under the experiment before this task's meter, plus
    /// this task's settled spend.
    experiment_spent: u64,
    task_spent: u64,
    calls: u32,
    reserved: BTreeMap<u64, u64>,
    next: u64,
}

impl Meter {
    /// A task's meter, starting from what the experiment has spent so far.
    pub fn new(ceilings: Ceilings, experiment_spent: u64) -> Self {
        Self {
            ceilings,
            experiment_spent,
            task_spent: 0,
            calls: 0,
            reserved: BTreeMap::new(),
            next: 0,
        }
    }

    /// Hold `amount` for a call about to start, or say why it may not.
    /// A free call is still counted against `task_calls` and stopped by a
    /// spent task; only the experiment ceiling lets free calls go on.
    pub fn reserve(&mut self, amount: u64) -> Result<Ticket, Stop> {
        let held: u64 = self.reserved.values().sum();
        let in_flight = u32::try_from(self.reserved.len()).unwrap_or(u32::MAX);
        if self.calls.saturating_add(in_flight) >= self.ceilings.task_calls {
            return Err(Stop::TaskCalls);
        }
        if self.task_spent >= self.ceilings.task {
            return Err(Stop::TaskSpend);
        }
        if amount > 0 {
            let over = |spent: u64, ceiling: u64| {
                spent.saturating_add(held).saturating_add(amount) > ceiling
            };
            if over(self.task_spent, self.ceilings.task) {
                return Err(Stop::TaskSpend);
            }
            if over(self.experiment_spent, self.ceilings.experiment) {
                return Err(Stop::Experiment);
            }
        }
        let id = self.next;
        self.next += 1;
        self.reserved.insert(id, amount);
        Ok(Ticket(id))
    }

    /// Replace a reservation with what the call cost. A call that failed
    /// before reaching the provider settles at 0 but still counts as a call.
    pub fn settle(&mut self, ticket: Ticket, actual: u64) {
        if self.reserved.remove(&ticket.0).is_none() {
            return;
        }
        self.calls = self.calls.saturating_add(1);
        self.task_spent = self.task_spent.saturating_add(actual);
        self.experiment_spent = self.experiment_spent.saturating_add(actual);
    }

    /// Settled spend for this task, micro-dollars.
    pub fn task_spent(&self) -> u64 {
        self.task_spent
    }

    /// Settled spend for the experiment, micro-dollars.
    pub fn experiment_spent(&self) -> u64 {
        self.experiment_spent
    }

    /// Settled calls for this task.
    pub fn calls(&self) -> u32 {
        self.calls
    }

    /// Held for calls in flight, micro-dollars. After a crash this is lost
    /// with the process; the caller persists reservations so a restart errs
    /// toward stopping.
    pub fn reserved(&self) -> u64 {
        self.reserved.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USD: u64 = 1_000_000;
    const CEIL: Ceilings = Ceilings {
        experiment: 200 * USD,
        task: 25 * USD,
        task_calls: 400,
    };
    /// $3 in, $15 out, $0.30 cached, per million tokens.
    const PAID: Price = Price {
        input: 3 * USD,
        output: 15 * USD,
        cached_input: 300_000,
    };

    #[test]
    fn cost_bills_cached_input_apart_and_rounds_up() {
        let u = Usage {
            input_tokens: 1_000,
            output_tokens: 100,
            cached_input_tokens: 800,
        };
        // 200 × $3/M = 600 µ$; 800 × $0.30/M = 240 µ$; 100 × $15/M = 1500 µ$.
        assert_eq!(cost(&u, &PAID), 600 + 240 + 1_500);
        let one = Usage {
            input_tokens: 1,
            ..Usage::default()
        };
        let cheap = Price {
            input: 1,
            ..Price::default()
        };
        assert_eq!(
            cost(&one, &cheap),
            1,
            "a millionth of a micro-dollar rounds up"
        );
        assert_eq!(cost(&u, &Price::default()), 0);
    }

    #[test]
    fn worst_case_counts_every_byte_and_output_token() {
        // 4,000 bytes = 1,000 tokens × $3/M = 3,000 µ$; 1,000 out × $15/M = 15,000 µ$.
        // (4,000 + 1,024) × $3/M = 15,072 µ$; 1,000 × $15/M = 15,000 µ$;
        // plus 1 µ$ of rounding slack.
        assert_eq!(worst_case(4_000, 1_000, &PAID), 30_073);
        assert_eq!(
            worst_case(4_001, 0, &PAID),
            charge(4_001 + FRAMING_TOKENS, PAID.input) + 1
        );
    }

    #[test]
    fn worst_case_covers_dense_prompts_and_framing() {
        // A prompt billed at one token per byte plus framing, with every
        // output token used, never costs more than was reserved.
        for bytes in [0, 1, 999, 4_000, 120_000] {
            let billed = Usage {
                input_tokens: bytes + FRAMING_TOKENS,
                output_tokens: 2_048,
                cached_input_tokens: 0,
            };
            assert!(
                cost(&billed, &PAID) <= worst_case(bytes, 2_048, &PAID),
                "{bytes}"
            );
        }
    }

    #[test]
    fn worst_case_covers_any_cached_split_and_rounding() {
        let prices = [
            PAID,
            // Cached dearer than fresh.
            Price {
                input: 0,
                output: 0,
                cached_input: USD,
            },
            // Tiny rates where per-bucket rounding matters.
            Price {
                input: 2,
                output: 0,
                cached_input: 1,
            },
            Price {
                input: 1,
                output: 1,
                cached_input: 1,
            },
        ];
        for p in prices {
            for bytes in [0, 1, 4, 8, 999] {
                let tokens = bytes + FRAMING_TOKENS;
                let bound = worst_case(bytes, 7, &p);
                for cached in [0, 1, tokens / 2, tokens - 1, tokens] {
                    let u = Usage {
                        input_tokens: tokens,
                        output_tokens: 7,
                        cached_input_tokens: cached,
                    };
                    assert!(cost(&u, &p) <= bound, "{p:?} {bytes} {cached}");
                }
            }
        }
    }

    #[test]
    fn a_spent_task_stops_free_calls_too() {
        let mut m = Meter::new(
            Ceilings {
                experiment: 100,
                task: 10,
                task_calls: 50,
            },
            0,
        );
        let t = m.reserve(10).expect("fits");
        m.settle(t, 10);
        assert_eq!(m.reserve(0), Err(Stop::TaskSpend));
    }

    #[test]
    fn parallel_reservations_cannot_pass_the_task_ceiling() {
        let mut m = Meter::new(CEIL, 0);
        let a = m.reserve(10 * USD).expect("first");
        let _b = m.reserve(10 * USD).expect("second");
        assert_eq!(m.reserve(6 * USD), Err(Stop::TaskSpend));
        assert!(
            m.reserve(5 * USD).is_ok(),
            "exactly at the ceiling is allowed"
        );
        m.settle(a, 2 * USD);
        assert_eq!(m.task_spent(), 2 * USD);
        assert_eq!(m.reserved(), 15 * USD);
    }

    #[test]
    fn the_experiment_ceiling_stops_paid_calls_only() {
        let mut m = Meter::new(CEIL, 199 * USD);
        assert_eq!(m.reserve(2 * USD), Err(Stop::Experiment));
        assert!(m.reserve(0).is_ok(), "a free local call still runs");
    }

    #[test]
    fn calls_are_capped_even_when_free() {
        let mut m = Meter::new(
            Ceilings {
                task_calls: 2,
                ..CEIL
            },
            0,
        );
        let a = m.reserve(0).expect("one");
        let _b = m.reserve(0).expect("two in flight");
        assert_eq!(m.reserve(0), Err(Stop::TaskCalls));
        m.settle(a, 0);
        assert_eq!(m.calls(), 1);
        assert_eq!(
            m.reserve(0),
            Err(Stop::TaskCalls),
            "one settled, one in flight"
        );
    }

    #[test]
    fn an_unsettled_reservation_stays_charged() {
        let mut m = Meter::new(CEIL, 0);
        let _lost = m.reserve(20 * USD).expect("held");
        assert_eq!(m.reserve(6 * USD), Err(Stop::TaskSpend));
    }
}
