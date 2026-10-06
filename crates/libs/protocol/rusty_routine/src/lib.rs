//! Routines: an AG-UI agent run on a schedule.
//!
//! A [`Routine`] is a cron [`Schedule`], a prompt and a failure budget.
//! When it is due, [`Routine::fire`] builds the [`RunAgentInput`] to post:
//! a fresh thread per firing carrying the prompt as the person's message,
//! with the routine's name and the scheduled time in `forwardedProps` so a
//! gateway rule can tell a routine's run from a person's. [`Routine::record`]
//! counts the outcome: a run that fails raises the count, a run that
//! succeeds resets it, and the routine is disabled when the count reaches
//! its budget rather than failing on every tick until someone notices.
//!
//! Sans-IO, like `rusty_channel`: nothing here reads a clock, sleeps or
//! opens a socket. The `run` feature adds [`run`], a loop that does.
//!
//! Step 6 of the ADR-0007 follow-ons.

#[cfg(feature = "run")]
pub mod run;
pub mod schedule;

use std::fmt;

use rusty_agui::{Content, Message, RunAgentInput};
use rusty_json::Value;

pub use schedule::Schedule;

/// What can go wrong reading routines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A cron expression that does not parse.
    Schedule(String),
    /// A routines document that is not what [`load`] expects.
    Config(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Schedule(why) => write!(f, "schedule: {why}"),
            Error::Config(why) => write!(f, "config: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// Default failure budget when a routine names none.
pub const DEFAULT_MAX_FAILURES: u32 = 3;

/// One scheduled run of an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routine {
    /// Unique among the routines a runner holds; the thread id prefix.
    pub name: String,
    /// When it fires, in UTC.
    pub schedule: Schedule,
    /// The person's message every run starts with.
    pub prompt: String,
    /// Consecutive failures after which the routine is disabled.
    pub max_failures: u32,
    /// The next firing, seconds since the epoch; `None` once disabled.
    pub next_run: Option<u64>,
    /// Failures in a row so far.
    pub failures: u32,
}

impl Routine {
    /// A routine first due at the schedule's next time after `now`.
    pub fn new(
        name: impl Into<String>,
        schedule: Schedule,
        prompt: impl Into<String>,
        max_failures: u32,
        now: u64,
    ) -> Self {
        let next_run = schedule.next_after(now);
        Routine {
            name: name.into(),
            schedule,
            prompt: prompt.into(),
            max_failures,
            next_run,
            failures: 0,
        }
    }

    /// Disabled: no `next_run`, by the failure budget or an exhausted
    /// schedule.
    pub fn disabled(&self) -> bool {
        self.next_run.is_none()
    }

    /// Whether `now` has reached the next firing.
    pub fn due(&self, now: u64) -> bool {
        self.next_run.is_some_and(|at| at <= now)
    }

    /// The run for a firing at `now`, and the schedule advanced past it.
    /// A firing that was missed (the runner slept through it) is not made
    /// up: one run per wake, the next from the schedule after `now`.
    pub fn fire(&mut self, now: u64) -> RunAgentInput {
        let scheduled = self.next_run.unwrap_or(now);
        self.next_run = self.schedule.next_after(now);
        let mut props = Value::Object(Default::default());
        let _ = props.insert("routine", Value::String(self.name.clone()));
        let _ = props.insert("scheduledAt", Value::from(scheduled as f64));
        let mut input = RunAgentInput::new(
            format!("routine:{}:{scheduled}", self.name),
            format!("routine:{}:{now}", self.name),
            vec![Message::User {
                id: format!("prompt:{scheduled}"),
                content: Content::Text(self.prompt.clone()),
                name: Some("routine".into()),
            }],
        );
        input.forwarded_props = props;
        input
    }

    /// Count a run's outcome. `max_failures` in a row disables the routine.
    pub fn record(&mut self, succeeded: bool) {
        if succeeded {
            self.failures = 0;
            return;
        }
        self.failures += 1;
        if self.failures >= self.max_failures {
            self.next_run = None;
        }
    }
}

/// Read routines from a JSON array of `{"name", "cron", "prompt",
/// "maxFailures"?}`. Names must be unique.
pub fn load(json: &str, now: u64) -> Result<Vec<Routine>, Error> {
    let value = Value::parse(json).map_err(|e| Error::Config(e.to_string()))?;
    let items = value
        .as_array()
        .ok_or_else(|| Error::Config("expected an array of routines".into()))?;
    let mut routines: Vec<Routine> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let field = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Config(format!("routine {i}: no string {key:?}")))
        };
        let name = field("name")?;
        if routines.iter().any(|r| r.name == name) {
            return Err(Error::Config(format!(
                "routine {i}: duplicate name {name:?}"
            )));
        }
        let schedule = Schedule::parse(field("cron")?)?;
        let max_failures = match item.get("maxFailures") {
            None => DEFAULT_MAX_FAILURES,
            Some(v) => v
                .as_i64()
                .filter(|n| *n >= 1)
                .map(|n| n as u32)
                .ok_or_else(|| {
                    Error::Config(format!(
                        "routine {i}: maxFailures must be a positive integer"
                    ))
                })?,
        };
        routines.push(Routine::new(
            name,
            schedule,
            field("prompt")?,
            max_failures,
            now,
        ));
    }
    Ok(routines)
}

/// The earliest next firing among `routines`, if any is still enabled.
pub fn next_due(routines: &[Routine]) -> Option<u64> {
    routines.iter().filter_map(|r| r.next_run).min()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_700_000_000; // 2023-11-14 22:13:20 UTC

    fn hourly(now: u64) -> Routine {
        Routine::new(
            "digest",
            Schedule::parse("0 * * * *").expect("cron"),
            "What's due?",
            2,
            now,
        )
    }

    #[test]
    fn a_routine_fires_on_schedule_with_a_fresh_thread() {
        let mut r = hourly(T0);
        let first = 1_700_002_800; // 23:00
        assert_eq!(r.next_run, Some(first));
        assert!(!r.due(first - 1) && r.due(first) && r.due(first + 30));

        let input = r.fire(first + 30);
        assert_eq!(input.thread_id, format!("routine:digest:{first}"));
        assert_eq!(input.messages.len(), 1);
        assert!(
            matches!(&input.messages[0], Message::User { content: Content::Text(t), .. } if t == "What's due?")
        );
        assert_eq!(
            input.forwarded_props.get("routine").and_then(Value::as_str),
            Some("digest")
        );
        assert_eq!(
            input
                .forwarded_props
                .get("scheduledAt")
                .and_then(Value::as_f64),
            Some(first as f64)
        );
        assert_eq!(r.next_run, Some(first + 3600), "advanced past the firing");

        // A missed firing is skipped, not made up.
        let input = r.fire(first + 3 * 3600 + 5);
        assert_eq!(input.thread_id, format!("routine:digest:{}", first + 3600));
        assert_eq!(r.next_run, Some(first + 4 * 3600));
    }

    #[test]
    fn failures_in_a_row_disable_and_a_success_resets() {
        let mut r = hourly(T0);
        r.record(false);
        assert!(!r.disabled() && r.failures == 1);
        r.record(true);
        assert_eq!(r.failures, 0);
        r.record(false);
        r.record(false);
        assert!(r.disabled(), "two in a row with a budget of two");
        assert!(!r.due(u64::MAX));
    }

    #[test]
    fn routines_load_from_json_and_bad_ones_are_named() {
        let routines = load(
            r#"[{"name":"digest","cron":"0 9 * * 1-5","prompt":"What's due today?"},
                {"name":"ping","cron":"*/5 * * * *","prompt":"ping","maxFailures":1}]"#,
            T0,
        )
        .expect("loads");
        assert_eq!(routines.len(), 2);
        assert_eq!(routines[0].max_failures, DEFAULT_MAX_FAILURES);
        assert_eq!(routines[1].max_failures, 1);
        assert_eq!(
            next_due(&routines),
            Some(1_700_000_100),
            "the five-minute one is first: 22:15"
        );

        for (json, word) in [
            (r#"{}"#, "array"),
            (r#"[{"cron":"* * * * *","prompt":"x"}]"#, "name"),
            (r#"[{"name":"a","cron":"x","prompt":"x"}]"#, "schedule"),
            (
                r#"[{"name":"a","cron":"* * * * *","prompt":"x","maxFailures":0}]"#,
                "positive",
            ),
            (
                r#"[{"name":"a","cron":"* * * * *","prompt":"x"},{"name":"a","cron":"* * * * *","prompt":"y"}]"#,
                "duplicate",
            ),
        ] {
            let err = load(json, T0).expect_err(json).to_string();
            assert!(err.contains(word), "{err} should mention {word}");
        }
        assert_eq!(next_due(&[]), None);
    }
}
