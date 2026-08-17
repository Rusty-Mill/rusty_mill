//! Outbound webhook dispatch: a minimal, secret-scrubbed POST on
//! `NeedsInput`/`Finished`/`Errored`/`SubagentFinished`.
//!
//! Configured via `SESSIONMGR_WEBHOOK_URL`, matching `SESSIONMGR_HOME`'s
//! own env-var-based configuration convention (`paths::HOME_ENV`).
//! Unset means nothing is sent -- not an error, just nothing configured.
//!
//! **Secret-scrubbing is the payload shape itself**, per PLAN.md: no
//! transcript content (sidesteps the redaction problem rather than
//! half-solving it), no command line (an initial prompt can carry a
//! pasted secret), and the worktree path is relative to the session's
//! own repository root, never absolute -- an absolute Windows path
//! leaks the local username (adversarial finding #11).

use std::time::Duration;

use serde::Serialize;
use sessionmgr_core::{Session, SessionKind, SessionStatus};

pub const WEBHOOK_URL_ENV: &str = "SESSIONMGR_WEBHOOK_URL";

/// A best-effort webhook call gets this long before it is abandoned.
/// Generous for a notification nobody is blocked waiting on, bounded so
/// a slow or dead endpoint can never accumulate unboundedly many
/// lingering threads.
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize)]
struct Payload {
    session_id: String,
    event: &'static str,
    kind: &'static str,
    status: &'static str,
    branch: Option<String>,
    /// Relative to the session's own repository root -- never absolute.
    worktree_path: Option<String>,
}

/// Fires a webhook for `event` if `SESSIONMGR_WEBHOOK_URL` is set.
///
/// Best-effort, deliberately: runs on its own throwaway thread and never
/// returns anything the caller could block on or fail because of. A
/// session's own operation must never depend on webhook delivery
/// succeeding, or a dead/slow notification endpoint would degrade the
/// actual product.
pub fn notify(session: &Session, event: &'static str) {
    let Ok(url) = std::env::var(WEBHOOK_URL_ENV) else {
        return;
    };
    let worktree_path = session.workspace.as_ref().and_then(|w| {
        w.cwd
            .strip_prefix(&w.repo)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    });
    let payload = Payload {
        session_id: session.id.to_string(),
        event,
        kind: kind_name(session.kind),
        status: status_name(session.status),
        branch: session.workspace.as_ref().and_then(|w| w.branch.clone()),
        worktree_path,
    };
    let body = match serde_json::to_vec(&payload) {
        Ok(body) => body,
        Err(e) => {
            eprintln!("sessionmgr: could not build webhook payload: {e}");
            return;
        }
    };
    std::thread::spawn(move || {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .build();
        let agent: ureq::Agent = config.into();
        if let Err(e) = agent
            .post(&url)
            .header("Content-Type", "application/json")
            .send(&body)
        {
            eprintln!("sessionmgr: webhook delivery failed: {e}");
        }
    });
}

fn kind_name(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::PlainTerminal => "terminal",
        SessionKind::SameDirectory => "same-dir",
        SessionKind::Worktree => "worktree",
        SessionKind::Dependent => "dependent",
    }
}

fn status_name(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Created => "created",
        SessionStatus::Waiting => "waiting",
        SessionStatus::Running => "running",
        SessionStatus::NeedsInput => "needs-input",
        SessionStatus::Finished => "finished",
        SessionStatus::Errored => "errored",
        SessionStatus::Crashed => "crashed",
        SessionStatus::Closed => "closed",
        SessionStatus::Merged => "merged",
        SessionStatus::Discarded => "discarded",
    }
}
