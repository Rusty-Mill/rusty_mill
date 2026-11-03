//! `rusty-mux-ctl agent` as installed: the hook it writes runs this binary,
//! and that hook speaks only inside a block.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CTL: &str = env!("CARGO_BIN_EXE_rusty-mux-ctl");

fn ctl(args: &[&str]) -> Output {
    Command::new(CTL)
        .args(args)
        .env_remove("RUSTY_MUX_BLOCK")
        .env_remove("RUSTY_MUX_WINDOW")
        .env_remove("RUSTY_MUX_SESSION")
        .output()
        .expect("runs")
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rusty-mux-ctl-agent-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

/// The command of the one SessionStart hook in `dir`'s settings.
fn hook_command(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join("settings.json")).expect("settings");
    let settings = rusty_json::Value::from_json_str(&text).expect("JSON");
    settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .expect("a command")
        .to_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn setup_status_and_remove_round_trip_through_the_binary() {
    let dir = scratch();
    let dir_arg = dir.to_str().expect("UTF-8 temp dir");
    let setup = ctl(&["agent", "setup", "--dir", dir_arg]);
    assert!(setup.status.success(), "{setup:?}");

    let command = hook_command(&dir);
    assert!(command.ends_with(" agent hook"), "{command}");
    #[cfg(unix)]
    {
        // Run as Claude Code runs it: through the shell, in a block.
        let block = "00000000-0000-0000-0000-00000000000c";
        let ran = Command::new("/bin/sh")
            .args(["-c", &command])
            .env("RUSTY_MUX_BLOCK", block)
            .output()
            .expect("sh");
        assert!(stdout(&ran).contains(block), "{ran:?}");
    }

    let status = ctl(&["--json", "agent", "status", "--dir", dir_arg]);
    assert!(status.status.success(), "{status:?}");
    assert_eq!(stdout(&status).matches(r#""state":"installed""#).count(), 2);

    let remove = ctl(&["agent", "remove", "--dir", dir_arg]);
    assert!(remove.status.success(), "{remove:?}");
    assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 0);
    assert_eq!(
        ctl(&["agent", "status", "--dir", dir_arg]).status.code(),
        Some(1)
    );
}

#[test]
fn the_hook_is_silent_outside_a_block_and_names_it_inside() {
    let outside = ctl(&["agent", "hook"]);
    assert!(outside.status.success());
    assert!(outside.stdout.is_empty());

    let block = "00000000-0000-0000-0000-00000000000b";
    let inside = Command::new(CTL)
        .args(["agent", "hook"])
        .env("RUSTY_MUX_BLOCK", block)
        .output()
        .expect("runs");
    assert!(inside.status.success());
    assert!(stdout(&inside).contains(block), "{inside:?}");
}
