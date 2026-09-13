//! Regression test for the child-process-hang bug in
//! `ClaudeCliBackend::chat`: before the fix, a wedged `claude -p` process
//! meant `wait_with_output()` never returned and `Engine::train` blocked
//! forever with no way to recover.
//!
//! `ClaudeCliBackend` hardcodes the program name `claude` (resolved via
//! `PATH`), so there's no constructor seam to swap the binary directly.
//! Instead this prepends a scratch directory containing this crate's own
//! `hang_forever` test helper -- built alongside this test and located via
//! Cargo's `CARGO_BIN_EXE_<name>` mechanism -- installed under the name
//! `claude` (with the platform's executable extension). That's the same
//! `PATH`-injection technique `claude_cli.rs`'s own
//! `chat_reports_a_clear_error_when_claude_is_missing_from_path` unit test
//! already uses to control which `claude` binary gets resolved.

use std::time::{Duration, Instant};

use skillopt_core::{ChatBackend, Message};
use skillopt_model::ClaudeCliBackend;

#[tokio::test]
async fn chat_times_out_instead_of_hanging_forever_on_a_wedged_claude_process() {
    let scratch = tempfile::tempdir().expect("tempdir for the fake `claude` binary");
    let target_name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let target = scratch.path().join(target_name);
    std::fs::copy(env!("CARGO_BIN_EXE_hang_forever"), &target)
        .expect("copy hang_forever stand-in into place as `claude`");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms).unwrap();
    }

    // PATH is process-global and Rust runs tests in the same process
    // concurrently by default -- restored on every exit path below, mirroring
    // `claude_cli.rs`'s own PATH-mutating unit test.
    let original_path = std::env::var_os("PATH");
    let mut new_path = std::ffi::OsString::from(scratch.path());
    let sep = if cfg!(windows) { ";" } else { ":" };
    if let Some(existing) = &original_path {
        new_path.push(sep);
        new_path.push(existing);
    }
    std::env::set_var("PATH", &new_path);

    let backend =
        ClaudeCliBackend::new("sonnet".to_string()).with_timeout(Duration::from_millis(200));
    let started = Instant::now();
    let result = backend.chat(&[Message::user("hi")]).await;
    let elapsed = started.elapsed();

    if let Some(original) = original_path {
        std::env::set_var("PATH", original);
    } else {
        std::env::remove_var("PATH");
    }

    let err =
        result.expect_err("a permanently wedged `claude` process must time out, not hang forever");
    assert!(
        err.to_string().contains("did not finish within"),
        "expected a timeout error, got: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "chat() took {elapsed:?} -- the 200ms timeout should have fired well under 10s"
    );
}
