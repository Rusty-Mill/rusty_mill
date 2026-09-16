//! Exercises the compiled `arun` binary itself — not the library code
//! in isolation — so this test fails if the async wiring breaks between
//! `main` and the library, not just within it.

#![cfg(target_os = "linux")]

use std::process::Command;

fn arun() -> Command {
    Command::new(env!("CARGO_BIN_EXE_arun"))
}

#[test]
fn arun_propagates_success_exit_code() {
    let status = arun().arg("/bin/true").status().expect("run arun");
    assert_eq!(status.code(), Some(0));
}

#[test]
fn arun_propagates_failure_exit_code() {
    let status = arun().arg("/bin/false").status().expect("run arun");
    assert_eq!(status.code(), Some(1));
}

#[test]
fn arun_reports_usage_with_no_arguments() {
    let status = arun().status().expect("run arun");
    assert_eq!(status.code(), Some(2));
}

#[test]
fn arun_passes_arguments_through() {
    let output = arun()
        .args(["/bin/echo", "hello", "async", "world"])
        .output()
        .expect("run arun");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello async world\n");
}
