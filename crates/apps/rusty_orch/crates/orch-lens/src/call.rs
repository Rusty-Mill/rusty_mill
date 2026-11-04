//! One job's call: fit the context to each provider in the job's fallback
//! order, reserve its worst case, ask, settle what it cost. Pure over the
//! [`Model`] port and the [`Meter`]; the shell supplies both.
//!
//! - A provider that is not built, whose window the context cannot fit,
//!   that the experiment ceiling stops (free providers go on), or that
//!   answers `Unavailable` is passed over, with the reason recorded.
//! - `Permanent` and `Transient` end the job on that provider: a bad reply
//!   is not retried elsewhere, and a retry is the scheduler's call.
//! - A task ceiling ends the job before any call.
//! - A reply using more output tokens than allowed is refused as
//!   `Permanent`; its cost is still settled.
//! - The meter is saved after every reservation, before the provider is
//!   asked, and again after it settles. A reservation that cannot be saved
//!   is released and ends the job: no call goes out unrecorded. A
//!   settlement that cannot be saved ends the fallback too, so no further
//!   provider is asked before the shell sees it.

use std::collections::BTreeMap;

use orch_core::AgentId;
use orch_dispatch::{AgentError, ClassifiedError};

use crate::config::{Provider, Tools};
use crate::context::{assemble, Parts, Refusal, Window};
use crate::meter::{cost, worst_case, Meter, Stop};
use crate::port::{Model, Question};
use crate::post::Post;
use crate::schedule::Job;

/// Output tokens one lens reply may use, on every provider.
pub const MAX_OUTPUT_TOKENS: u32 = 4_096;

/// The built adapters, by provider id.
pub trait Models {
    /// Provider `id`'s adapter for a lens with `tools`, or `None` when it
    /// was not built.
    fn get(&mut self, id: AgentId, tools: Tools) -> Option<&mut dyn Model>;
}

/// How a job's call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    Answered {
        provider: AgentId,
        posts: Vec<Post>,
    },
    /// `Permanent` or `Transient` from `provider`.
    Failed {
        provider: AgentId,
        error: ClassifiedError,
    },
    /// A ceiling stops the job; no call was made after it.
    Stopped(Stop),
    /// Every provider was passed over.
    Exhausted,
    /// The meter could not be saved: a reservation (released, nothing
    /// asked) or a passed-over provider's settlement (no further provider
    /// asked).
    Unsaved(String),
}

/// A job's call: the providers passed over, and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Called {
    pub passed: Vec<(AgentId, String)>,
    pub end: End,
    /// A settlement that could not be saved. The saved reservation then
    /// settles at its worst case on restart; the shell should stop.
    pub unsaved: Option<String>,
}

/// What a call is billed to and sized by: the task, the configured
/// providers and the experiment's one meter.
pub struct Ctx<'a> {
    pub task: &'a str,
    pub providers: &'a BTreeMap<AgentId, Provider>,
    pub meter: &'a mut Meter,
    /// Persists the meter; called after each reserve and each settle.
    pub save: &'a mut dyn FnMut(&Meter) -> Result<(), String>,
}

/// Run `job` with `parts` (its phase and suffix are taken from the job).
pub fn call(job: &Job, parts: &Parts<'_>, mut ctx: Ctx<'_>, models: &mut dyn Models) -> Called {
    let parts = Parts {
        phase: job.phase,
        suffix: job.suffix,
        ..*parts
    };
    let mut passed = Vec::new();
    let mut experiment = false;
    let mut unsaved = None;
    for &id in &job.providers {
        match attempt(job, &parts, id, &mut ctx, models, &mut unsaved) {
            Step::Pass(reason) => {
                passed.push((id, reason));
                if let Some(e) = &unsaved {
                    let end = End::Unsaved(e.clone());
                    return Called {
                        passed,
                        end,
                        unsaved,
                    };
                }
            }
            Step::Experiment => {
                experiment = true;
                passed.push((id, Stop::Experiment.to_string()));
            }
            Step::End(end) => {
                return Called {
                    passed,
                    end,
                    unsaved,
                }
            }
        }
    }
    let end = if experiment {
        End::Stopped(Stop::Experiment)
    } else {
        End::Exhausted
    };
    Called {
        passed,
        end,
        unsaved,
    }
}

enum Step {
    Pass(String),
    Experiment,
    End(End),
}

fn attempt(
    job: &Job,
    parts: &Parts<'_>,
    id: AgentId,
    ctx: &mut Ctx<'_>,
    models: &mut dyn Models,
    unsaved: &mut Option<String>,
) -> Step {
    let providers = ctx.providers;
    let Some(provider) = providers.get(&id) else {
        return Step::Pass("not configured".to_owned());
    };
    let window = Window {
        context_tokens: provider.context_tokens,
        max_output_tokens: MAX_OUTPUT_TOKENS,
        clearance: provider.max_class,
    };
    let (prefix, body) = match assemble(parts, window) {
        Ok(text) => text,
        Err(refusal) => return Step::Pass(refused(&refusal)),
    };
    let Some(model) = models.get(id, job.tools) else {
        return Step::Pass("not built".to_owned());
    };
    let bytes = (prefix.len() + body.len()) as u64;
    let held = worst_case(bytes, MAX_OUTPUT_TOKENS, &provider.price);
    let ticket = match ctx.meter.reserve(ctx.task, held) {
        Ok(t) => t,
        Err(Stop::Experiment) => return Step::Experiment,
        Err(stop) => return Step::End(End::Stopped(stop)),
    };
    if let Err(e) = (ctx.save)(ctx.meter) {
        ctx.meter.release(ticket);
        return Step::End(End::Unsaved(e));
    }
    let settle = |amount: u64| {
        ctx.meter.settle(ticket, amount);
        if let Err(e) = (ctx.save)(ctx.meter) {
            unsaved.get_or_insert(e);
        }
    };
    let question = Question {
        prefix,
        body,
        allowed: job.allowed.clone(),
        max_output_tokens: MAX_OUTPUT_TOKENS,
    };
    match model.ask(&question) {
        Ok(answer) => {
            settle(cost(&answer.usage, &provider.price));
            if answer.usage.output_tokens > u64::from(MAX_OUTPUT_TOKENS) {
                let error = ClassifiedError::Permanent(AgentError(format!(
                    "reply used {} output tokens, over {MAX_OUTPUT_TOKENS}",
                    answer.usage.output_tokens
                )));
                return Step::End(End::Failed {
                    provider: id,
                    error,
                });
            }
            Step::End(End::Answered {
                provider: id,
                posts: answer.posts,
            })
        }
        Err(ClassifiedError::Unavailable(e)) => {
            settle(0);
            Step::Pass(format!("unavailable: {e}"))
        }
        // Usage is unknown: settle the worst case, so a restart errs
        // toward stopping.
        Err(error) => {
            settle(held);
            Step::End(End::Failed {
                provider: id,
                error,
            })
        }
    }
}

fn refused(r: &Refusal) -> String {
    match r {
        Refusal::AboveClearance { card, clearance } => {
            format!("claim is {card:?}, provider cleared for {clearance:?}")
        }
        Refusal::TooLarge { needed, room } => {
            format!("context needs {needed} bytes, window holds {room}")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use rusty_bbp::{MsgId, PrincipalId};
    use rusty_json::Map;

    use super::*;
    use crate::card::fixtures::{card, item};
    use crate::class::DataClass;
    use crate::context::Phase;
    use crate::meter::{Ceilings, Price};
    use crate::port::{Answer, Usage};
    use crate::post::Allowed;
    use crate::schedule::{JobKey, Slot};
    use rusty_bbp::Direction;

    const T: &str = "task-1";
    const USD: u64 = 1_000_000;
    const PAID: Price = Price {
        input: 3 * USD,
        output: 15 * USD,
        cached_input: 300_000,
    };

    fn id(s: &str) -> AgentId {
        AgentId::new(s).expect("id")
    }

    fn provider(context_tokens: u32, price: Price) -> Provider {
        Provider {
            kind: crate::config::ProviderKind::OpenAiHttp,
            max_class: DataClass::Internal,
            context_tokens,
            price,
            settings: Map::new(),
        }
    }

    fn providers(list: &[(&str, Provider)]) -> BTreeMap<AgentId, Provider> {
        list.iter().map(|(n, p)| (id(n), p.clone())).collect()
    }

    fn meter(experiment: u64, task_calls: u32) -> Meter {
        Meter::new(
            Ceilings {
                experiment,
                task: 25 * USD,
                task_calls,
            },
            0,
        )
    }

    #[derive(Default)]
    struct Fake {
        replies: VecDeque<Result<Answer, ClassifiedError>>,
        asked: Vec<Question>,
    }

    impl Model for Fake {
        fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
            self.asked.push(q.clone());
            self.replies
                .pop_front()
                .unwrap_or_else(|| Err(ClassifiedError::Permanent(AgentError("no reply".into()))))
        }
    }

    struct Fakes(BTreeMap<AgentId, Fake>);

    impl Models for Fakes {
        fn get(&mut self, id: AgentId, _: Tools) -> Option<&mut dyn Model> {
            self.0.get_mut(&id).map(|f| f as &mut dyn Model)
        }
    }

    impl Fakes {
        fn new(list: Vec<(&str, Vec<Result<Answer, ClassifiedError>>)>) -> Self {
            Self(
                list.into_iter()
                    .map(|(n, r)| {
                        let fake = Fake {
                            replies: r.into(),
                            asked: Vec::new(),
                        };
                        (id(n), fake)
                    })
                    .collect(),
            )
        }

        fn asked(&self, n: &str) -> &[Question] {
            &self.0[&id(n)].asked
        }
    }

    fn answer(output_tokens: u64) -> Result<Answer, ClassifiedError> {
        Ok(Answer {
            posts: Vec::new(),
            usage: Usage {
                input_tokens: 1_000,
                output_tokens,
                cached_input_tokens: 0,
            },
        })
    }

    fn err(f: fn(AgentError) -> ClassifiedError) -> Result<Answer, ClassifiedError> {
        Err(f(AgentError("down".into())))
    }

    fn job(order: &[&str]) -> Job {
        Job {
            key: JobKey {
                lens: "adversary".to_owned(),
                claim: MsgId(7),
                slot: Slot::Review,
            },
            phase: Phase::Blind,
            providers: order.iter().map(|n| id(n)).collect(),
            allowed: Allowed {
                evidence: None,
                claims: None,
            },
            suffix: Some("Look harder."),
            foreign_seen: 0,
            tools: Tools::None,
        }
    }

    struct Run {
        called: Called,
        fakes: Fakes,
        meter: Meter,
    }

    fn run(job: &Job, cfg: &BTreeMap<AgentId, Provider>, meter: Meter, fakes: Fakes) -> Run {
        run_saving(job, cfg, meter, fakes, &mut |_| Ok(()))
    }

    fn run_saving(
        job: &Job,
        cfg: &BTreeMap<AgentId, Provider>,
        mut meter: Meter,
        mut fakes: Fakes,
        save: &mut dyn FnMut(&Meter) -> Result<(), String>,
    ) -> Run {
        let c = card(vec![item(
            20,
            "operator",
            Direction::Counter,
            DataClass::Internal,
        )]);
        let lens = PrincipalId("adversary".to_owned());
        let parts = Parts {
            prefix: "# Intent\nx\n",
            card: &c,
            lens: &lens,
            phase: Phase::Informed,
            lines: &[],
            record: None,
            suffix: None,
        };
        let ctx = Ctx {
            task: T,
            providers: cfg,
            meter: &mut meter,
            save,
        };
        let called = call(job, &parts, ctx, &mut fakes);
        Run {
            called,
            fakes,
            meter,
        }
    }

    const FREE: Price = Price {
        input: 0,
        output: 0,
        cached_input: 0,
    };

    #[test]
    fn unavailable_moves_to_the_next_provider() {
        let cfg = providers(&[("a", provider(32_768, FREE)), ("b", provider(32_768, FREE))]);
        let fakes = Fakes::new(vec![
            ("a", vec![err(ClassifiedError::Unavailable)]),
            ("b", vec![answer(10)]),
        ]);
        let r = run(&job(&["a", "b"]), &cfg, meter(USD, 10), fakes);
        assert_eq!(
            r.called.end,
            End::Answered {
                provider: id("b"),
                posts: Vec::new()
            }
        );
        assert_eq!(
            r.called.passed,
            vec![(id("a"), "unavailable: down".to_owned())]
        );
        assert_eq!(r.meter.calls(T), 2, "both calls count");
    }

    #[test]
    fn permanent_and_transient_do_not_fall_back() {
        for f in [ClassifiedError::Permanent, ClassifiedError::Transient] {
            let cfg = providers(&[("a", provider(32_768, FREE)), ("b", provider(32_768, FREE))]);
            let fakes = Fakes::new(vec![("a", vec![err(f)]), ("b", vec![answer(10)])]);
            let r = run(&job(&["a", "b"]), &cfg, meter(USD, 10), fakes);
            assert_eq!(
                r.called.end,
                End::Failed {
                    provider: id("a"),
                    error: f(AgentError("down".into()))
                }
            );
            assert!(r.fakes.asked("b").is_empty(), "b is never asked");
        }
    }

    #[test]
    fn a_context_too_large_for_one_provider_moves_to_the_next() {
        let cfg = providers(&[
            ("a", provider(MAX_OUTPUT_TOKENS + 10, FREE)),
            ("b", provider(32_768, FREE)),
        ]);
        let fakes = Fakes::new(vec![("a", vec![answer(10)]), ("b", vec![answer(10)])]);
        let r = run(&job(&["a", "b"]), &cfg, meter(USD, 10), fakes);
        assert!(matches!(r.called.end, End::Answered { provider, .. } if provider == id("b")));
        assert!(
            r.called.passed[0].1.starts_with("context needs"),
            "{:?}",
            r.called.passed
        );
        assert!(r.fakes.asked("a").is_empty());
        assert_eq!(r.meter.calls(T), 1, "a refused fit is not a call");
    }

    #[test]
    fn unknown_or_unbuilt_providers_are_passed_over() {
        let cfg = providers(&[("a", provider(32_768, FREE))]);
        let r = run(
            &job(&["ghost", "a"]),
            &cfg,
            meter(USD, 10),
            Fakes::new(vec![]),
        );
        assert_eq!(r.called.end, End::Exhausted);
        assert_eq!(
            r.called.passed,
            vec![
                (id("ghost"), "not configured".to_owned()),
                (id("a"), "not built".to_owned())
            ]
        );
    }

    #[test]
    fn the_experiment_ceiling_stops_paid_calls_and_lets_free_ones_go_on() {
        let cfg = providers(&[
            ("paid", provider(32_768, PAID)),
            ("free", provider(32_768, FREE)),
        ]);
        let fakes = Fakes::new(vec![("paid", vec![answer(10)]), ("free", vec![answer(10)])]);
        let r = run(&job(&["paid", "free"]), &cfg, meter(0, 10), fakes);
        assert!(matches!(r.called.end, End::Answered { provider, .. } if provider == id("free")));
        assert!(r.fakes.asked("paid").is_empty());

        let fakes = Fakes::new(vec![("paid", vec![answer(10)])]);
        let r = run(&job(&["paid"]), &cfg, meter(0, 10), fakes);
        assert_eq!(r.called.end, End::Stopped(Stop::Experiment));
        assert_eq!(r.meter.calls(T), 0);
    }

    #[test]
    fn a_task_ceiling_ends_the_job_before_any_call() {
        let cfg = providers(&[("a", provider(32_768, FREE)), ("b", provider(32_768, FREE))]);
        let fakes = Fakes::new(vec![("a", vec![answer(10)]), ("b", vec![answer(10)])]);
        let r = run(&job(&["a", "b"]), &cfg, meter(USD, 0), fakes);
        assert_eq!(r.called.end, End::Stopped(Stop::TaskCalls));
        assert!(r.fakes.asked("a").is_empty() && r.fakes.asked("b").is_empty());
    }

    #[test]
    fn an_over_long_reply_is_refused_and_still_paid_for() {
        let cfg = providers(&[("paid", provider(32_768, PAID))]);
        let over = u64::from(MAX_OUTPUT_TOKENS) + 1;
        let fakes = Fakes::new(vec![("paid", vec![answer(over)])]);
        let r = run(&job(&["paid"]), &cfg, meter(100 * USD, 10), fakes);
        assert!(
            matches!(&r.called.end, End::Failed { error: ClassifiedError::Permanent(e), .. } if e.0.contains("output tokens")),
            "{:?}",
            r.called.end
        );
        let usage = Usage {
            input_tokens: 1_000,
            output_tokens: over,
            cached_input_tokens: 0,
        };
        assert_eq!(r.meter.task_spent(T), cost(&usage, &PAID));
    }

    #[test]
    fn a_failure_with_unknown_usage_settles_the_worst_case() {
        let cfg = providers(&[("paid", provider(32_768, PAID))]);
        let fakes = Fakes::new(vec![("paid", vec![err(ClassifiedError::Transient)])]);
        let r = run(&job(&["paid"]), &cfg, meter(100 * USD, 10), fakes);
        let q = &r.fakes.asked("paid")[0];
        let held = worst_case(
            (q.prefix.len() + q.body.len()) as u64,
            MAX_OUTPUT_TOKENS,
            &PAID,
        );
        assert!(held > 0);
        assert_eq!(r.meter.task_spent(T), held);
        assert_eq!(r.meter.reserved(), 0, "nothing left held");
    }

    #[test]
    fn the_job_sets_phase_suffix_and_output_limit() {
        let cfg = providers(&[("a", provider(32_768, FREE))]);
        let fakes = Fakes::new(vec![("a", vec![answer(10)])]);
        let r = run(&job(&["a"]), &cfg, meter(USD, 10), fakes);
        let q = &r.fakes.asked("a")[0];
        assert!(
            !q.body.contains("note 20"),
            "blind hides foreign evidence: {}",
            q.body
        );
        assert!(q.body.contains("Look harder."), "{}", q.body);
        assert_eq!(q.max_output_tokens, MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn the_reservation_is_saved_before_the_provider_is_asked() {
        let cfg = providers(&[("paid", provider(32_768, PAID))]);
        let fakes = Fakes::new(vec![("paid", vec![answer(10)])]);
        let mut held = Vec::new();
        let r = run_saving(
            &job(&["paid"]),
            &cfg,
            meter(100 * USD, 10),
            fakes,
            &mut |m| {
                held.push((m.reserved(), m.calls(T)));
                Ok(())
            },
        );
        assert!(matches!(r.called.end, End::Answered { .. }));
        assert_eq!(held.len(), 2, "after reserve and after settle");
        assert!(
            held[0].0 > 0 && held[0].1 == 0,
            "held, not yet a call: {held:?}"
        );
        assert_eq!(held[1], (0, 1), "settled");
        assert_eq!(r.called.unsaved, None);
    }

    #[test]
    fn an_unsaved_reservation_is_released_and_nothing_is_asked() {
        let cfg = providers(&[("a", provider(32_768, FREE)), ("b", provider(32_768, FREE))]);
        let fakes = Fakes::new(vec![("a", vec![answer(10)]), ("b", vec![answer(10)])]);
        let r = run_saving(&job(&["a", "b"]), &cfg, meter(USD, 10), fakes, &mut |_| {
            Err("disk full".to_owned())
        });
        assert_eq!(r.called.end, End::Unsaved("disk full".to_owned()));
        assert!(r.fakes.asked("a").is_empty() && r.fakes.asked("b").is_empty());
        assert_eq!((r.meter.reserved(), r.meter.calls(T)), (0, 0));
    }

    #[test]
    fn an_unsaved_settlement_keeps_the_answer_and_is_reported() {
        let cfg = providers(&[("a", provider(32_768, FREE))]);
        let fakes = Fakes::new(vec![("a", vec![answer(10)])]);
        let mut n = 0;
        let r = run_saving(&job(&["a"]), &cfg, meter(USD, 10), fakes, &mut |_| {
            n += 1;
            if n == 1 {
                Ok(())
            } else {
                Err("disk full".to_owned())
            }
        });
        assert!(matches!(r.called.end, End::Answered { .. }));
        assert_eq!(r.called.unsaved, Some("disk full".to_owned()));
    }

    #[test]
    fn an_unsaved_settlement_stops_the_fallback() {
        let cfg = providers(&[("a", provider(32_768, FREE)), ("b", provider(32_768, FREE))]);
        let fakes = Fakes::new(vec![
            ("a", vec![err(ClassifiedError::Unavailable)]),
            ("b", vec![answer(10)]),
        ]);
        let mut n = 0;
        let r = run_saving(&job(&["a", "b"]), &cfg, meter(USD, 10), fakes, &mut |_| {
            n += 1;
            if n == 2 {
                Err("disk full".to_owned())
            } else {
                Ok(())
            }
        });
        assert_eq!(r.called.end, End::Unsaved("disk full".to_owned()));
        assert_eq!(r.called.unsaved, Some("disk full".to_owned()));
        assert!(r.fakes.asked("b").is_empty(), "no provider after it");
    }
}
