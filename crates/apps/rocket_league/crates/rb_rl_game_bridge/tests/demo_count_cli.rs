//! Synthetic CLI failure checks; never reads private recordings or starts the game.
#![allow(clippy::unwrap_used)]

#[test]
fn truncated_recording_returns_failure_without_a_lifecycle_report() {
    let path = std::env::temp_dir().join(format!("rb-demo-truncated-{}.bin", std::process::id()));
    std::fs::write(&path, [0, 3, 1, 2]).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rb_rl_demo_count"))
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("rb_rl_demo_count:"));
}
