//! A runner: fire due routines against one agent, forever.
//!
//! One run at a time; the loop sleeps until the earliest next firing. A
//! run succeeds when the stream ends in `RUN_FINISHED` with no error, and
//! what the agent said goes to `out`, one line per routine. Routines that
//! exhaust their failure budget are disabled for the life of the process;
//! state is not persisted.

use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusty_agui::HttpAgent;
use rusty_channel::Thread;

use crate::{next_due, Routine};

/// Seconds since the epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Fire every routine that is due at `now` and report each outcome.
/// Returns how many ran.
pub fn tick(routines: &mut [Routine], agent: &HttpAgent, now: u64, out: &mut dyn Write) -> usize {
    let mut ran = 0;
    for routine in routines.iter_mut().filter(|r| r.due(now)) {
        ran += 1;
        let input = routine.fire(now);
        let mut thread = Thread {
            id: input.thread_id.clone(),
            messages: input.messages.clone(),
        };
        let outcome = match agent.run(&input) {
            Ok(stream) => {
                let reply = thread.absorb(stream);
                match reply.error {
                    None => Ok(reply.text),
                    Some(e) => Err(format!("{e} (after: {})", reply.text)),
                }
            }
            Err(e) => Err(e.to_string()),
        };
        routine.record(outcome.is_ok());
        let line = match &outcome {
            Ok(text) => format!("[{}] ok: {}", routine.name, text.replace('\n', " ")),
            Err(e) if routine.disabled() => format!(
                "[{}] failed: {e}; disabled after {} failures",
                routine.name, routine.failures
            ),
            Err(e) => format!(
                "[{}] failed ({}/{}): {e}",
                routine.name, routine.failures, routine.max_failures
            ),
        };
        let _ = writeln!(out, "{line}");
    }
    ran
}

/// Run until every routine is disabled.
pub fn run(mut routines: Vec<Routine>, agent: &HttpAgent, out: &mut dyn Write) {
    loop {
        let Some(due) = next_due(&routines) else {
            let _ = writeln!(out, "every routine is disabled; stopping");
            return;
        };
        let t = now();
        if due > t {
            std::thread::sleep(Duration::from_secs(due - t));
            continue;
        }
        tick(&mut routines, agent, t, out);
    }
}
