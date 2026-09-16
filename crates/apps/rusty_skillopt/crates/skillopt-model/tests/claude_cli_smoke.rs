//! Exercises `ClaudeCliBackend` against a real `claude` CLI session --
//! the actual subprocess boundary, not a stand-in for it. `#[ignore]`d by
//! default (like this crate's other environment-dependent smoke tests)
//! since it needs `claude` on PATH and authenticated, which CI doesn't
//! have.
//!
//! Run with:
//!   cargo test -p skillopt-model --test claude_cli_smoke -- --ignored
//!
//! Verified live in this project's own development sandbox: raw HTTPS to
//! api.anthropic.com 401s with no ANTHROPIC_API_KEY, but `claude -p`
//! already has a working session there -- this is the first real
//! (non-mock, non-wire-level) model call verified anywhere in this
//! project's history.

use skillopt_core::{ChatBackend, Message};
use skillopt_model::ClaudeCliBackend;

#[tokio::test]
#[ignore]
async fn reaches_a_real_claude_session_and_gets_back_the_requested_reply() {
    let backend = ClaudeCliBackend::new("sonnet".to_string());
    let messages = [
        Message::system(
            "You are a terse assistant. Reply with only the requested word, nothing else.",
        ),
        Message::user("Reply with exactly the word: verified"),
    ];

    let reply = backend
        .chat(&messages)
        .await
        .expect("a working `claude` CLI session should answer this trivially");

    assert!(
        reply.to_lowercase().contains("verified"),
        "expected the model to echo back \"verified\", got: {reply:?}"
    );
}
