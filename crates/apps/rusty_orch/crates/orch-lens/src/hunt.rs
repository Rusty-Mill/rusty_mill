//! One alien-lens hunt (#708, design D75 §3), pure over the [`Model`] port,
//! the meter and a [`Probes`] port that runs reproducers.
//!
//! Enumerate (one raw call) → [`parse_suspects`] → [`rank`] → per kept
//! suspect one hunt call → [`parse_probe`] → [`Probes::run`] on the
//! candidate and on base → [`outcome`]. The result is one [`HuntRow`],
//! saved whether or not anything was found, and one [`ProbeRow`] per probe
//! that actually ran. A probe that did not run (the port refused it, or
//! the runs could not be classified) gets no row: an unrun probe is not an
//! `error` (D78), so it never skews the §8 numbers.
//!
//! [`Model`]: crate::port::Model

use orch_core::AgentId;
use rusty_bbp::{MsgId, Sha256};

use crate::alien::{
    outcome, parse_probe, parse_suspects, rank, unavailability, HuntRow, Mechanism, ProbeRow, Run,
    Suspect,
};
use crate::call::{ask_raw, Ctx, Models, Raw, RawCalled, RawEnd};
use crate::class::DataClass;
use crate::config::Tools;
use crate::meter::Stop;

/// The candidate one hunt is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target<'a> {
    pub task: &'a str,
    pub claim: MsgId,
    /// The candidate artifact hunted.
    pub candidate: u64,
    pub claim_text: &'a str,
    /// The candidate's diffs, as text.
    pub diff: &'a str,
    /// Notes of the evidence already on the card: the available set.
    pub notes: &'a [String],
    /// The highest data class among the claim, its notes and the diff.
    pub class: DataClass,
    /// When the hunt began, Unix milliseconds.
    pub started: u64,
}

/// Who the hunt's calls go to: the alien lens's providers, in order, its
/// tool access and its cacheable prefix.
#[derive(Debug, Clone, Copy)]
pub struct Seat<'a> {
    pub providers: &'a [AgentId],
    pub tools: Tools,
    pub prefix: &'a str,
}

/// One reproducer to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe<'a> {
    /// Its integration-test file name, `probe_<claim>_<n>.rs`.
    pub file: String,
    /// The site it targets, which names the crate it belongs to.
    pub site: &'a str,
    pub source: &'a str,
}

/// How a probe ran: on the candidate, then on base when the candidate
/// failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Runs {
    pub candidate: Run,
    pub base: Option<Run>,
}

/// Runs reproducers in a sandbox (design §6). The shell supplies it.
pub trait Probes {
    /// Build and run `probe` on the candidate tree and, when it fails
    /// there, on base. `Err` means the probe did not run.
    fn run(&mut self, probe: &Probe<'_>) -> Result<Runs, String>;
}

/// A hunt that is recorded: the hunt row, the probes that ran, and a line
/// per step that did not go to plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunted {
    pub hunt: HuntRow,
    pub probes: Vec<ProbeRow>,
    pub notes: Vec<String>,
}

/// Why a hunt left nothing to record. Try again later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unrecorded {
    /// A ceiling stopped the enumerate call before it was made.
    Stopped(Stop),
    /// No provider could take the enumerate call.
    Exhausted(Vec<(AgentId, String)>),
    /// The meter could not be saved; the shell should stop.
    Unsaved(String),
}

/// Run one hunt on `target`. Once the enumerate call has been made, the
/// hunt is recorded even if a later step fails or a ceiling stops it, so
/// the calls already spent are not spent again.
///
/// # Errors
///
/// [`Unrecorded`] when nothing was spent on the hunt, or the meter could
/// not be saved.
pub fn hunt(
    target: &Target<'_>,
    seat: Seat<'_>,
    list: &[Mechanism],
    mut ctx: Ctx<'_>,
    models: &mut dyn Models,
    probes: &mut dyn Probes,
) -> Result<Hunted, Unrecorded> {
    let spent_before = ctx.meter.task_spent(target.task);
    let mut tokens = 0;
    let mut notes = Vec::new();
    let body = enumerate_body(target, list);
    let called = ask(seat, target.class, &body, &mut ctx, models)?;
    let suspects = match called.end {
        RawEnd::Answered {
            text, tokens: t, ..
        } => {
            tokens += t;
            parse_suspects(&text, list).unwrap_or_else(|e| {
                notes.push(format!("enumerate: {e}"));
                Vec::new()
            })
        }
        RawEnd::Stopped(stop) => return Err(Unrecorded::Stopped(stop)),
        RawEnd::Exhausted => return Err(Unrecorded::Exhausted(called.passed)),
        RawEnd::Unsaved(e) => return Err(Unrecorded::Unsaved(e)),
        RawEnd::Failed { provider, error } => {
            notes.push(format!("enumerate: {provider}: {error:?}"));
            Vec::new()
        }
    };
    let named = suspects.len();
    let mut rows = Vec::new();
    for (n, s) in (1u32..).zip(rank(suspects, target.notes, list)) {
        let body = hunt_body(target, &s, list);
        let text = match ask(seat, target.class, &body, &mut ctx, models)?.end {
            RawEnd::Answered {
                text, tokens: t, ..
            } => {
                tokens += t;
                text
            }
            RawEnd::Unsaved(e) => return Err(Unrecorded::Unsaved(e)),
            RawEnd::Stopped(stop) => {
                notes.push(format!("hunt {n}: stopped: {stop}"));
                break;
            }
            other => {
                notes.push(format!("hunt {n}: {other:?}"));
                continue;
            }
        };
        match parse_probe(&text) {
            Ok(Some(source)) => match probe(target, n, &s, &source, list, probes) {
                Ok(row) => rows.push(row),
                Err(e) => notes.push(format!("probe {n}: not run: {e}")),
            },
            Ok(None) => {}
            Err(e) => notes.push(format!("hunt {n}: {e}")),
        }
    }
    let hunt = HuntRow {
        task: target.task.to_owned(),
        claim: target.claim.0,
        candidate: target.candidate,
        suspects: u32::try_from(named).unwrap_or(u32::MAX),
        probes: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        tokens,
        cost: ctx
            .meter
            .task_spent(target.task)
            .saturating_sub(spent_before),
        started: target.started,
    };
    Ok(Hunted {
        hunt,
        probes: rows,
        notes,
    })
}

fn ask(
    seat: Seat<'_>,
    class: DataClass,
    body: &str,
    ctx: &mut Ctx<'_>,
    models: &mut dyn Models,
) -> Result<RawCalled, Unrecorded> {
    let raw = Raw {
        providers: seat.providers,
        tools: seat.tools,
        class,
        prefix: seat.prefix,
        body,
    };
    let sub = Ctx {
        task: ctx.task,
        providers: ctx.providers,
        meter: &mut *ctx.meter,
        save: &mut *ctx.save,
    };
    let called = ask_raw(&raw, sub, models);
    match called.unsaved {
        Some(e) => Err(Unrecorded::Unsaved(e)),
        None => Ok(called),
    }
}

/// Run probe `n` and classify it.
fn probe(
    target: &Target<'_>,
    n: u32,
    s: &Suspect,
    source: &str,
    list: &[Mechanism],
    probes: &mut dyn Probes,
) -> Result<ProbeRow, String> {
    let file = format!("probe_{}_{n}.rs", target.claim.0);
    let runs = probes.run(&Probe {
        file,
        site: &s.site,
        source,
    })?;
    let outcome = outcome(runs.candidate, runs.base)?;
    Ok(ProbeRow {
        task: target.task.to_owned(),
        claim: target.claim.0,
        candidate: target.candidate,
        site: s.site.clone(),
        mechanism: s.mechanism.to_owned(),
        novel: unavailability(s, target.notes, list) == 2,
        outcome,
        runs: 1 + u32::from(runs.base.is_some()),
        probe_sha256: Sha256::of(source.as_bytes()).hex(),
    })
}

/// The enumerate step's question body (design §3.2).
pub fn enumerate_body(target: &Target<'_>, list: &[Mechanism]) -> String {
    let mut out = String::from(
        "List up to 12 suspected defects in this candidate that ordinary review is unlikely to find.\n\
         Each suspect is a site in the diff (a crate::module::fn or a file path, never a bare name) \
         crossed with one mechanism id below, a one-line sketch of the failure, and your coherence \
         rating from 1 (a guess) to 5 (near certain).\n\nMechanisms:\n",
    );
    for m in list {
        out.push_str(&format!("- {}: {}\n", m.id, m.look_for));
    }
    out.push_str(&format!("\nClaim:\n{}\n", target.claim_text));
    out.push_str("\nEvidence already posted (do not repeat it):\n");
    if target.notes.is_empty() {
        out.push_str("(none)\n");
    }
    for n in target.notes {
        out.push_str(&format!("- {n}\n"));
    }
    out.push_str(&format!("\nDiff:\n{}\n", target.diff));
    out.push_str(
        "\nReply with JSON only: {\"suspects\": [{\"site\": \"...\", \"mechanism\": \"...\", \
         \"sketch\": \"...\", \"coherence\": 1}]}\n",
    );
    out
}

/// One hunt step's question body (design §3.4).
pub fn hunt_body(target: &Target<'_>, s: &Suspect, list: &[Mechanism]) -> String {
    let look_for = list
        .iter()
        .find(|m| m.id == s.mechanism)
        .map_or("", |m| m.look_for);
    format!(
        "Write one Rust integration test that asserts the correct behaviour at {site} and fails \
         if this suspected defect is real: {mechanism} ({look_for}): {sketch}\n\
         It reaches only the crate's public API, needs no network and no files outside a temp \
         directory, and finishes in seconds. If the defect cannot be shown that way, write \
         nothing.\n\nClaim:\n{claim}\n\nDiff:\n{diff}\n\n\
         Reply with JSON only: {{\"probe\": \"<test file source>\"}} or {{\"probe\": null}}\n",
        site = s.site,
        mechanism = s.mechanism,
        sketch = s.sketch,
        claim = target.claim_text,
        diff = target.diff,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};

    use orch_dispatch::{AgentError, ClassifiedError};
    use rusty_json::Map;

    use super::*;
    use crate::alien::{Outcome, RUST};
    use crate::config::{Provider, ProviderKind};
    use crate::meter::{Ceilings, Meter, Price};
    use crate::port::{Answer, Model, Question, Reply, Usage};

    const T: &str = "task-1";

    fn id(s: &str) -> AgentId {
        AgentId::new(s).expect("id")
    }

    /// Answers raw questions from a script, in order.
    struct Script(VecDeque<Result<String, ClassifiedError>>, Vec<Question>);

    impl Model for Script {
        fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
            self.1.push(q.clone());
            let next = self.0.pop_front().expect("scripted reply");
            next.map(|text| Answer {
                reply: Reply::Raw(text),
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cached_input_tokens: 0,
                },
            })
        }
    }

    struct One(Script);

    impl Models for One {
        fn get(&mut self, _: AgentId, _: Tools) -> Option<&mut dyn Model> {
            Some(&mut self.0)
        }
    }

    /// Runs every probe to the scripted result, recording what it saw.
    struct Fake(Result<Runs, String>, Vec<String>);

    impl Probes for Fake {
        fn run(&mut self, p: &Probe<'_>) -> Result<Runs, String> {
            self.1.push(p.file.clone());
            self.0.clone()
        }
    }

    fn providers() -> BTreeMap<AgentId, Provider> {
        let p = Provider {
            kind: ProviderKind::ClaudeCli,
            max_class: DataClass::Internal,
            context_tokens: 200_000,
            price: Price {
                input: 1_000_000,
                output: 1_000_000,
                cached_input: 0,
            },
            settings: Map::new(),
        };
        [(id("claude"), p)].into_iter().collect()
    }

    fn meter(task_calls: u32) -> Meter {
        Meter::new(
            Ceilings {
                experiment: 1_000_000_000,
                task: 1_000_000_000,
                task_calls,
            },
            0,
        )
    }

    const NOTES: &[String] = &[];

    fn target() -> Target<'static> {
        Target {
            task: T,
            claim: MsgId(7),
            candidate: 3,
            claim_text: "Write the store atomically",
            diff: "--- a/src/store.rs\n+++ b/src/store.rs\n",
            notes: NOTES,
            class: DataClass::Internal,
            started: 42,
        }
    }

    const SUSPECTS: &str = r#"{"suspects": [
        {"site": "store::write", "mechanism": "rs-atomic", "sketch": "no fsync", "coherence": 4},
        {"site": "store::read", "mechanism": "rs-panic", "sketch": "unwrap on bad file", "coherence": 3}
    ]}"#;
    const PROBE: &str = r##"{"probe": "#[test] fn t() {}"}"##;
    const NOTHING: &str = r#"{"probe": null}"#;

    fn run(
        replies: Vec<Result<&str, ClassifiedError>>,
        probes: &mut Fake,
        task_calls: u32,
    ) -> (Result<Hunted, Unrecorded>, Vec<Question>) {
        let ps = providers();
        let ids = [id("claude")];
        let seat = Seat {
            providers: &ids,
            tools: Tools::None,
            prefix: "prefix",
        };
        let mut m = meter(task_calls);
        let mut save = |_: &Meter| Ok(());
        let ctx = Ctx {
            task: T,
            providers: &ps,
            meter: &mut m,
            save: &mut save,
        };
        let script = replies.into_iter().map(|r| r.map(str::to_owned)).collect();
        let mut models = One(Script(script, Vec::new()));
        let out = hunt(&target(), seat, RUST, ctx, &mut models, probes);
        (out, models.0 .1)
    }

    fn fails_then(base: Run) -> Fake {
        Fake(
            Ok(Runs {
                candidate: Run::Failed,
                base: Some(base),
            }),
            Vec::new(),
        )
    }

    #[test]
    fn a_hunt_ranks_suspects_runs_probes_and_classifies_them() {
        let mut probes = fails_then(Run::Passed);
        let (out, asked) = run(vec![Ok(SUSPECTS), Ok(PROBE), Ok(NOTHING)], &mut probes, 10);
        let h = out.expect("recorded");
        assert_eq!(asked.len(), 3, "enumerate, then one hunt per suspect");
        assert!(asked.iter().all(|q| q.shape == crate::port::Shape::Raw));
        assert!(asked[0].body.contains("rs-atomic: "));
        assert!(asked[1].body.contains("store::write"), "rank put it first");
        assert_eq!(probes.1, ["probe_7_1.rs"]);
        assert_eq!(
            (
                h.hunt.suspects,
                h.hunt.probes,
                h.hunt.tokens,
                h.hunt.started
            ),
            (2, 1, 45, 42)
        );
        assert!(h.hunt.cost > 0, "the meter's spend is the hunt's cost");
        let p = &h.probes[0];
        assert_eq!((p.outcome, p.novel, p.runs), (Outcome::Regression, true, 2));
        assert_eq!(p.probe_sha256, Sha256::of(b"#[test] fn t() {}").hex());
        assert!(h.notes.is_empty(), "{:?}", h.notes);
    }

    #[test]
    fn a_hunt_with_nothing_found_is_still_recorded() {
        let mut probes = fails_then(Run::Passed);
        let (out, _) = run(vec![Ok(r#"{"suspects": []}"#)], &mut probes, 10);
        let h = out.expect("recorded");
        assert_eq!((h.hunt.suspects, h.hunt.probes), (0, 0));
        assert!(h.probes.is_empty() && probes.1.is_empty());
        let (out, _) = run(vec![Ok("not json")], &mut probes, 10);
        let h = out.expect("a bad enumerate reply still spent a call");
        assert_eq!(h.hunt.suspects, 0);
        assert!(h.notes[0].starts_with("enumerate: "), "{:?}", h.notes);
    }

    #[test]
    fn a_probe_that_did_not_run_gets_no_row() {
        let mut refused = Fake(Err("sandbox unavailable".to_owned()), Vec::new());
        let (out, _) = run(vec![Ok(SUSPECTS), Ok(PROBE), Ok(PROBE)], &mut refused, 10);
        let h = out.expect("recorded");
        assert_eq!((h.hunt.probes, h.probes.len()), (0, 0));
        assert_eq!(h.notes.len(), 2, "{:?}", h.notes);
        let mut unrun_base = Fake(
            Ok(Runs {
                candidate: Run::Failed,
                base: None,
            }),
            Vec::new(),
        );
        let (out, _) = run(
            vec![Ok(SUSPECTS), Ok(PROBE), Ok(NOTHING)],
            &mut unrun_base,
            10,
        );
        let h = out.expect("recorded");
        assert!(h.probes.is_empty(), "a failed candidate needs a base run");
        assert!(h.notes[0].contains("base was not run"), "{:?}", h.notes);
    }

    #[test]
    fn a_failed_hunt_call_skips_its_suspect_and_a_bad_probe_reply_is_noted() {
        let mut probes = fails_then(Run::Failed);
        let permanent = ClassifiedError::Permanent(AgentError("bad".to_owned()));
        let (out, _) = run(
            vec![Ok(SUSPECTS), Err(permanent), Ok(r#"{"probe": 3}"#)],
            &mut probes,
            10,
        );
        let h = out.expect("recorded");
        assert_eq!(h.probes.len(), 0);
        assert_eq!(h.notes.len(), 2, "{:?}", h.notes);
    }

    #[test]
    fn nothing_is_recorded_until_the_enumerate_call_is_made() {
        let mut probes = fails_then(Run::Passed);
        let (out, asked) = run(vec![], &mut probes, 0);
        assert!(matches!(out, Err(Unrecorded::Stopped(_))), "{out:?}");
        assert!(asked.is_empty());
        let unavailable = ClassifiedError::Unavailable(AgentError("down".to_owned()));
        let (out, _) = run(vec![Err(unavailable)], &mut probes, 10);
        assert!(matches!(out, Err(Unrecorded::Exhausted(_))), "{out:?}");
    }

    #[test]
    fn a_ceiling_mid_hunt_records_what_was_done() {
        let mut probes = fails_then(Run::Passed);
        // Two calls allowed: enumerate and the first hunt.
        let (out, asked) = run(vec![Ok(SUSPECTS), Ok(PROBE)], &mut probes, 2);
        let h = out.expect("recorded");
        assert_eq!(asked.len(), 2);
        assert_eq!((h.hunt.suspects, h.probes.len()), (2, 1));
        assert!(h.notes[0].starts_with("hunt 2: stopped"), "{:?}", h.notes);
    }
}
