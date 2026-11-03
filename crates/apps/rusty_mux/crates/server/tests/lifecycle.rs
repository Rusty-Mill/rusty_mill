//! Every block's program is reaped, however its block ends. A binary of its
//! own, because it counts this process's children.

#![cfg(target_os = "linux")]

mod common;

use std::time::{Duration, Instant};

use common::{Client, block_for, new_block, start_server_with, window_for};
use rusty_mux_proto::Message;
use rusty_mux_server::{Config, ConsoleHost};

const BLOCKS: usize = 8;

fn config(program: &str, args: &[&str]) -> Config {
    Config {
        program: program.into(),
        args: args.iter().map(Into::into).collect(),
        cwd: "/".into(),
        console_host: ConsoleHost::default(),
        config_file: None,
    }
}

/// Live or zombie children of this process, from `/proc/*/stat`.
fn children() -> Vec<u32> {
    let me = std::process::id();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| parent_of(*pid) == Some(me))
        .collect()
}

fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (comm) state ppid ...`; comm may hold spaces or parentheses.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

fn wait_for_no_children(case: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = children();
        if left.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{case}: children not reaped: {left:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn close_blocks_repeatedly(client: &mut Client) {
    for _ in 0..BLOCKS {
        let block = block_for(client);
        client.request(|req| Message::CloseBlock { req, block });
    }
}

/// One test, so the cases run in turn and the child count is theirs alone.
#[test]
fn every_closed_or_exited_block_is_reaped() {
    // A shell that exits on hangup.
    let mut client = Client::connect(&start_server_with(config("/bin/sh", &[])));
    close_blocks_repeatedly(&mut client);
    wait_for_no_children("CloseBlock of a shell");

    // A program that ignores SIGHUP is killed after the grace period.
    let stubborn = config("/bin/sh", &["-c", "trap '' HUP; exec sleep 1000"]);
    let mut client = Client::connect(&start_server_with(stubborn));
    close_blocks_repeatedly(&mut client);
    wait_for_no_children("CloseBlock of a program ignoring SIGHUP");

    // Programs that exit on their own.
    let mut client = Client::connect(&start_server_with(config("/bin/true", &[])));
    let window = window_for(&mut client);
    for _ in 0..BLOCKS {
        new_block(&mut client, window);
    }
    wait_for_no_children("a program that exits by itself");
}
