//! Regression test for the child-process-hang bug in
//! `AisfStageBackend::chat`: before the fix, a wedged AISF `eval-stage`
//! process meant `wait_with_output()` never returned and `Engine::train`
//! blocked forever with no way to recover.
//!
//! Swaps in `hang_forever` -- this crate's own test-only helper binary,
//! built alongside this test and located via Cargo's `CARGO_BIN_EXE_<name>`
//! mechanism -- as `AisfStageBackend`'s `binary_path`. That's the same
//! injection point `AisfStageBackend::new` already exposes for the real
//! AISF binary (see `tests/aisf_stage_smoke.rs`), so no test-only seam had
//! to be added to reach it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use skillopt_core::{prompts::executor_system_prompt, ChatBackend, Message, Skill};
use skillopt_model::AisfStageBackend;

#[tokio::test]
async fn chat_times_out_instead_of_hanging_forever_on_a_wedged_eval_stage_process() {
    let hang_forever = PathBuf::from(env!("CARGO_BIN_EXE_hang_forever"));
    let backend = AisfStageBackend::new(hang_forever, "triage".to_string())
        .with_timeout(Duration::from_millis(200));
    let skill = Skill::new("# Triage\n- Classify by user-facing impact.\n");
    let messages = [
        Message::system(executor_system_prompt(&skill)),
        Message::user(r#"{"issues": []}"#.to_string()),
    ];

    let started = Instant::now();
    let err = backend
        .chat(&messages)
        .await
        .expect_err("a permanently wedged eval-stage process must time out, not hang forever");
    let elapsed = started.elapsed();

    assert!(
        err.to_string().contains("did not finish within"),
        "expected a timeout error, got: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "chat() took {elapsed:?} -- the 200ms timeout should have fired well under 10s"
    );
}
