//! Exercises `AisfStageBackend` against a real, built AISF
//! `software-factory` binary -- the actual subprocess boundary, not a
//! stand-in for it. `#[ignore]`d by default (like `skillopt-model`'s own
//! MCP reference-server test) since it depends on a sibling checkout of
//! https://github.com/baileyrd/AISF existing on disk, not just this crate.
//!
//! Run with:
//!   AISF_BINARY_PATH=/path/to/aisf/target/debug/software-factory \
//!     cargo test -p skillopt-model --test aisf_stage_smoke -- --ignored
//!
//! No `ANTHROPIC_API_KEY` is required (and none is set here on purpose):
//! this only proves the wire format between the two projects is correct
//! (`FACTORY_PROMPTS_DIR` picked up, stdin scenario delivered, stdout/
//! stderr round-tripped) up to the point where AISF's `eval-stage` itself
//! needs a real model call -- that boundary is exactly where this test
//! stops, on purpose, rather than requiring live credentials to run at all.

use skillopt_core::{prompts::executor_system_prompt, ChatBackend, Message, Skill};
use skillopt_model::AisfStageBackend;

#[tokio::test]
#[ignore]
async fn reaches_a_real_eval_stage_binary_and_surfaces_its_error() {
    let binary_path = std::env::var("AISF_BINARY_PATH")
        .expect("set AISF_BINARY_PATH to a built AISF `software-factory` binary to run this test");

    let backend = AisfStageBackend::new(binary_path.into(), "triage".to_string());
    let skill = Skill::new("# Triage\n- Classify by user-facing impact.\n");
    let messages = [
        Message::system(executor_system_prompt(&skill)),
        Message::user(r#"{"issues": [{"number": 101, "title": "x", "labels": []}]}"#.to_string()),
    ];

    // ANTHROPIC_API_KEY deliberately unset -- confirms the whole path up to
    // AISF's own missing-key check works: our scratch dir was found via
    // FACTORY_PROMPTS_DIR, the scenario was delivered on stdin and parsed,
    // and eval-stage's own error made it back through stderr into ours.
    std::env::remove_var("ANTHROPIC_API_KEY");
    let err = backend.chat(&messages).await.unwrap_err();
    let msg = err.to_string();
    // AISF's own binary prints its Result::Err via Debug (Rust's default
    // `fn main() -> Result<...>` handler), so "MissingApiKey" (the enum
    // variant name), not its Display text, is what actually reaches stderr.
    assert!(
        msg.contains("MissingApiKey"),
        "expected AISF's own missing-API-key error to surface, got: {msg}"
    );
}
