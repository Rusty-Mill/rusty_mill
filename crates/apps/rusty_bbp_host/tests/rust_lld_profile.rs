//! Real resource-constrained Rust linking through the existing frozen profile/executor.
#![cfg(target_os = "linux")]
#![allow(clippy::expect_used, clippy::unwrap_used)]
use rusty_bbp::{Sha256, TaskId};
use rusty_bbp_host::{
    profiles::{self, ProfileSet},
    runner::Sandboxed,
};
use rusty_sandbox::{Executor, Limits, SandboxSpec};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn exercise(required: bool) {
    let root = std::env::temp_dir().join(format!(
        "bbp-rust-profile-{}-{required}",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join("work/tmp")).unwrap();
    let work = root.join("work").canonicalize().unwrap();
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let sysroot = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let set = ProfileSet::rust_lld_tests(&sysroot).unwrap();
    assert_eq!(set.limits.memory_bytes, 1 << 30);
    assert!(set.profiles[0]
        .args
        .iter()
        .any(|a| a == "link-arg=-Wl,--threads=1"));
    assert!(!set
        .read_roots
        .iter()
        .any(|r| r == "/" || r.ends_with("/.cargo")));
    let state = root.join("state");
    let task = TaskId("rust-profile".into());
    let digest = profiles::freeze(&state, &task, &set).unwrap();
    let frozen = profiles::load(&state, &task, digest).unwrap();
    assert_eq!(frozen.to_bytes().unwrap(), set.to_bytes().unwrap());
    std::fs::write(
        work.join("src.rs"),
        r#"
fn clamp(v: i64, lo: i64, hi: i64) -> Result<i64, &'static str> {
    if lo > hi { return Err("invalid bounds"); }
    Ok(v.max(lo).min(hi))
}
#[test] fn interior() { assert_eq!(clamp(3, 1, 5), Ok(3)); }
#[test] fn below() { assert_eq!(clamp(-10, 1, 5), Ok(1)); }
#[test] fn above() { assert_eq!(clamp(10, 1, 5), Ok(5)); }
#[test] fn equal() { assert_eq!(clamp(0, 7, 7), Ok(7)); }
#[test] fn invalid() { assert_eq!(clamp(0, 5, 1), Err("invalid bounds")); }
#[test] fn extremes() { assert_eq!(clamp(i64::MIN, i64::MIN, i64::MAX), Ok(i64::MIN)); }
#[test] fn no_overflow() { assert_eq!(clamp(i64::MAX, i64::MIN, 0), Ok(0)); }
"#,
    )
    .unwrap();
    std::fs::create_dir(work.join("src")).unwrap();
    std::fs::rename(work.join("src.rs"), work.join("src/lib.rs")).unwrap();
    let mut read: Vec<_> = frozen.read_roots.iter().map(PathBuf::from).collect();
    read.push(work.clone());
    let mut env = frozen.env.clone();
    env.push((
        "TMPDIR".into(),
        work.join("tmp").to_string_lossy().into_owned(),
    ));
    let limits = Limits::new(
        Duration::from_secs(10),
        Duration::from_secs(30),
        1 << 30,
        1 << 28,
        256,
        4096,
    )
    .unwrap();
    let spec = SandboxSpec::new(
        read,
        vec![work.clone(), "/dev/null".into()],
        work.clone(),
        env,
        limits,
    )
    .unwrap();
    let exec = Sandboxed::new(
        PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        &state.join("sandbox"),
    );
    for profile in &frozen.profiles {
        let outcome = exec.exec(&spec, &profile.program, &profile.args);
        if !required
            && outcome.as_ref().err().is_some_and(|e| {
                e.to_string()
                    .contains("Landlock filesystem confinement is NotEnforced")
            })
        {
            assert!(!work.join(".bbp-tests").exists());
            eprintln!(
                "Rust LLD execution unavailable on this kernel; verified fail-closed refusal"
            );
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        let outcome = outcome.expect("enforced sandbox required");
        assert!(
            outcome.termination.succeeded(),
            "{}: {} {}",
            profile.name,
            String::from_utf8_lossy(&outcome.stdout),
            String::from_utf8_lossy(&outcome.stderr)
        );
        if profile.name == "test" {
            assert!(String::from_utf8_lossy(&outcome.stdout).contains("7 passed; 0 failed"));
        }
    }
    let again = profiles::load(&state, &task, digest).unwrap();
    assert_eq!(Sha256::of(&again.to_bytes().unwrap()), digest);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rust_lld_profile_preserves_confinement_and_bounds_parallelism() {
    exercise(false);
}

#[test]
#[ignore = "requires enforced Linux sandbox and the Rust LLD toolchain"]
fn confined_rust_lld_profile_passes_under_one_gib() {
    exercise(true);
    println!("BBP_ACCEPTANCE PASS rust-lld");
}
