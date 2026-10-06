//! A bot really is confined: it writes in its workspace, cannot write
//! outside it, and dies as a group when stopped. Linux only, like the
//! sandbox; elsewhere `start` refuses and that is the test.

use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use rusty_bot::Fleet;
use rusty_bot::{default_limits, executor, start, BotSpec};

fn scratch(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("rusty-bot-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("work")).expect("workspace");
    base.canonicalize().expect("canonical")
}

fn shell_bot(base: &Path, name: &str, script: &str) -> BotSpec {
    BotSpec {
        name: name.into(),
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        workspace: base.join("work"),
        read_roots: ["/usr", "/lib", "/lib64", "/bin", "/etc"]
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect(),
        limits: default_limits(),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_bot_writes_only_in_its_workspace_and_stops_as_a_group() {
    let base = scratch("confined");
    let escaped = base.join("escaped");
    let script = format!(
        "echo hello > note.txt; touch {} 2>/dev/null; sleep 30 & sleep 30",
        escaped.display()
    );
    let spec = shell_bot(&base, "shell", &script);
    let executor = executor(
        &PathBuf::from(env!("CARGO_BIN_EXE_rusty-bot")),
        &base.join("state"),
    );

    let mut running = start(&executor, &spec).expect("the bot starts");
    std::thread::sleep(Duration::from_millis(400));
    if let Some(outcome) = running.outcome() {
        panic!("ended early: {outcome:?}");
    }
    assert_eq!(
        std::fs::read_to_string(base.join("work/note.txt"))
            .expect("written")
            .trim(),
        "hello",
        "the workspace is writable"
    );
    assert!(
        !escaped.exists(),
        "nothing outside the workspace was written"
    );

    running.stop().expect("stopped");
    let outcome = running.wait().expect("waited");
    assert_eq!(outcome.termination, rusty_sandbox::Termination::Signaled(9));
    assert!(outcome.wall < Duration::from_secs(10));
    std::fs::remove_dir_all(&base).expect("cleanup");
}

#[cfg(target_os = "linux")]
#[test]
fn a_fleet_is_all_up_or_all_down() {
    let base = scratch("fleet");
    let executor = executor(
        &PathBuf::from(env!("CARGO_BIN_EXE_rusty-bot")),
        &base.join("state"),
    );
    let good = shell_bot(&base, "good", "sleep 30");
    let mut bad = shell_bot(&base, "bad", "sleep 30");
    bad.workspace = "relative/path".into();

    let err =
        Fleet::start(&executor, &[good.clone(), bad]).expect_err("the bad spec fails the fleet");
    assert!(err.to_string().contains("\"bad\""), "{err}");

    let mut fleet = Fleet::start(&executor, &[good]).expect("a good fleet starts");
    assert!(fleet.any_running());
    fleet.stop();
    let outcome = fleet.bots.remove(0).wait().expect("waited");
    assert_eq!(outcome.termination, rusty_sandbox::Termination::Signaled(9));
    std::fs::remove_dir_all(&base).expect("cleanup");
}

#[cfg(not(target_os = "linux"))]
#[test]
fn off_linux_a_bot_is_refused_rather_than_run_unconfined() {
    let base = scratch("refused");
    let executor = executor(
        &PathBuf::from(env!("CARGO_BIN_EXE_rusty-bot")),
        &base.join("state"),
    );
    let spec = shell_bot(&base, "shell", "sleep 1");
    assert!(start(&executor, &spec).is_err());
}
