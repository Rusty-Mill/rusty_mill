//! Without `runtime::install_fallback`, rusty_tokio still refuses to run off
//! its own runtime. A separate binary from `tests/fallback.rs`, which
//! installs it for the whole process.

#[test]
#[should_panic(expected = "install_fallback")]
fn spawn_without_a_runtime_or_fallback_panics() {
    drop(rusty_tokio::spawn(async {}));
}

#[test]
#[should_panic(expected = "no rusty_tokio runtime")]
fn sleep_without_a_runtime_or_fallback_panics() {
    drop(rusty_tokio::time::sleep(std::time::Duration::from_millis(
        1,
    )));
}
