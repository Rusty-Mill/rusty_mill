//! `lel-jail` off Linux: the jail is Linux only (D85), so the launcher
//! refuses before reading anything, with the setup-failure code.
#![cfg(not(target_os = "linux"))]

#[test]
fn lel_jail_refuses_off_linux() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_lel-jail"))
        .arg("jail.json")
        .output()
        .expect("lel-jail");
    assert_eq!(
        out.status.code(),
        Some(i32::from(rusty_sandbox::SETUP_FAILED))
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("only implemented on Linux"), "{stderr}");
}
