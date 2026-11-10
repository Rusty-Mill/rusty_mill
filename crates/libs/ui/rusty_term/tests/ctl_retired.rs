//! `rusty_term ctl` retired with the GUI's local mode (D159): it names the
//! replacement and fails, with no `Error: ...` debug dump after it.

use std::process::Command;

#[test]
fn ctl_names_its_replacement_and_fails_cleanly() {
    let out = Command::new(env!("CARGO_BIN_EXE_rusty_term"))
        .args(["ctl", "list"])
        .output()
        .expect("run rusty_term");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("rusty-mux-ctl"), "{stderr}");
    assert!(!stderr.contains("Error:"), "{stderr}");
    assert!(out.stdout.is_empty());
}
