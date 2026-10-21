//! The parent's own credentials never reach a child, even when the caller
//! scrubs nothing. Its own test binary: it sets a process-wide variable.
#![cfg(unix)]

use std::time::Duration;

use orch_cli::{CommandRunner, StdCommand, BOARD_TOKEN_ENV};

#[test]
fn a_child_cannot_read_the_board_token() {
    std::env::set_var(BOARD_TOKEN_ENV, "sentinel-board-token");
    std::env::set_var("ORCH_CLI_VISIBLE", "visible");
    let script = format!("printf '%s|%s' \"${{{BOARD_TOKEN_ENV}-unset}}\" \"$ORCH_CLI_VISIBLE\"");
    let argv = ["/bin/sh".to_owned(), "-c".to_owned(), script];
    for scrub in [&[][..], &["OTHER"][..]] {
        let out = StdCommand
            .run_scrubbed(&argv, &[], Duration::from_secs(10), scrub)
            .expect("run");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "unset|visible");
    }
}
