//! `meshed --help` (CLI-002): runs the actual compiled `meshed` binary
//! (not just `clap`'s own `render_help()`, which `app::tests::help_output_lists_the_registered_subcommands`
//! already covers as a unit test) and checks its real process exit
//! code and stdout, the way a shell caller actually observes it.

use std::process::Command;

#[test]
fn help_exits_0_and_lists_health_and_metrics() {
    let output = Command::new(env!("CARGO_BIN_EXE_meshed"))
        .arg("--help")
        .output()
        .expect("failed to run the meshed binary");

    assert!(
        output.status.success(),
        "expected exit code 0, got {:?}",
        output.status.code()
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("health"), "help output: {stdout}");
    assert!(stdout.contains("metrics"), "help output: {stdout}");
}
