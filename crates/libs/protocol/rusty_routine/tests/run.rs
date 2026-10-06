//! The runner against a real agent on `rusty_serve`: one that answers, one
//! that fails. Needs the `run` feature.
#![cfg(feature = "run")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rusty_agui::{Agent, AgentHandler, Emitter, Error, HttpAgent, RunAgentInput};
use rusty_json::Value;
use rusty_routine::{run::tick, Routine, Schedule};

/// Echoes the prompt back, or fails, counting its runs.
struct Scripted {
    runs: Arc<AtomicUsize>,
    fail: bool,
}

impl Agent for Scripted {
    fn run(
        &mut self,
        input: &RunAgentInput,
        emit: &mut Emitter,
    ) -> rusty_agui::Result<Option<Value>> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(Error::Agent("the model is down".into()));
        }
        let routine = input
            .forwarded_props
            .get("routine")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let prompt = input
            .messages
            .first()
            .map(|m| match m {
                rusty_agui::Message::User { content, .. } => content.text(),
                _ => String::new(),
            })
            .unwrap_or_default();
        emit.text(&format!("{routine} asked: {prompt}"))?;
        Ok(None)
    }
}

fn serve(fail: bool) -> (HttpAgent, Arc<AtomicUsize>) {
    let runs = Arc::new(AtomicUsize::new(0));
    let server = rusty_serve::Server::bind(
        "127.0.0.1:0".parse().expect("addr"),
        AgentHandler::new(Scripted {
            runs: Arc::clone(&runs),
            fail,
        }),
    )
    .expect("bind");
    let addr = server.local_addr().expect("addr");
    std::thread::spawn(move || {
        let _ = server.run();
    });
    (
        HttpAgent::new(&format!("http://{addr}/api/agent")).expect("agent"),
        runs,
    )
}

const T0: u64 = 1_700_000_000;

fn routine(name: &str, max_failures: u32) -> Routine {
    Routine::new(
        name,
        Schedule::parse("* * * * *").expect("cron"),
        "what's due?",
        max_failures,
        T0,
    )
}

#[test]
fn due_routines_run_and_their_replies_are_reported() {
    let (agent, runs) = serve(false);
    let mut routines = vec![routine("digest", 3), routine("later", 3)];
    routines[1].next_run = Some(T0 + 3600);
    let mut out = Vec::new();

    assert_eq!(
        tick(&mut routines, &agent, T0 + 60, &mut out),
        1,
        "only the due one"
    );
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        String::from_utf8_lossy(&out).trim(),
        "[digest] ok: digest asked: what's due?"
    );
    // T0 is 22:13:20; the tick at 22:14:20 leaves the next firing at 22:15:00.
    assert_eq!(routines[0].next_run, Some(1_700_000_100));
    assert_eq!(routines[0].failures, 0);
}

#[test]
fn a_failing_agent_counts_down_the_budget_and_disables() {
    let (agent, runs) = serve(true);
    let mut routines = vec![routine("digest", 2)];
    let mut out = Vec::new();

    assert_eq!(tick(&mut routines, &agent, T0 + 60, &mut out), 1);
    assert_eq!(tick(&mut routines, &agent, T0 + 120, &mut out), 1);
    assert_eq!(
        tick(&mut routines, &agent, T0 + 180, &mut out),
        0,
        "disabled: nothing due"
    );
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("[digest] failed (1/2): the model is down"),
        "{text}"
    );
    assert!(text.contains("disabled after 2 failures"), "{text}");
    assert!(routines[0].disabled());
}
