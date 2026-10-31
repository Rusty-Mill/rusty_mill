//! How blocks end on Windows: a close reaches every program on the
//! console, a flooding block closes without stalling the server, and a
//! server that dies takes its blocks with it. A program that exits by
//! itself reports its exit code, and `terminal.process` names its pid.
//! Each runs on ConPTY and on our own console host.

#![cfg(windows)]

mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Client, PATIENCE, block_for, connect, run_and_wait, start_server_on};
use rusty_mux_core::{BlockId, Event, ExitStatus};
use rusty_mux_proto::schema::Value;
use rusty_mux_proto::{Features, Message};
use rusty_mux_server::ConsoleHost;

/// Prints the block's shell pid as `SHELL=<n>`: powershell's parent.
const SHELL_PID: &[u8] = b"powershell -NoProfile -c \"'SHELL=' + (Get-CimInstance Win32_Process -Filter ('ProcessId=' + $PID)).ParentProcessId\"\r";
/// Starts a ping attached to the block's console and prints its pid.
const PING: &[u8] = b"powershell -NoProfile -c \"$p = Start-Process ping -ArgumentList '-n','1000','127.0.0.1' -NoNewWindow -PassThru; 'PING=' + $p.Id\"\r";

/// Runs each scenario, a function of the host, once per host.
macro_rules! on_both_hosts {
    ($($scenario:ident),* $(,)?) => {$(
        mod $scenario {
            use super::ConsoleHost;

            #[test]
            fn conpty() {
                super::$scenario(ConsoleHost::ConPty);
            }

            #[test]
            fn own() {
                super::$scenario(ConsoleHost::Own);
            }
        }
    )*};
}

on_both_hosts!(
    closing_a_block_ends_every_program_on_its_console,
    closing_a_flooding_block_leaves_the_server_responsive,
    a_killed_server_takes_its_blocks_with_it,
    a_program_that_exits_reports_its_code,
    terminal_process_names_the_programs_pid
);

/// The `RUSTY_CONSOLE_HOST` value that names `host`.
fn console_host_name(host: ConsoleHost) -> &'static str {
    match host {
        ConsoleHost::ConPty => "conpty",
        ConsoleHost::Own => "own",
    }
}

fn shell_pid(client: &mut Client, block: BlockId) -> u32 {
    client.input(block, SHELL_PID);
    client.wait_for_number(block, "SHELL=")
}

/// Whether `tasklist` lists process `pid`: a listed process prints a CSV
/// line, which starts with a quote.
fn alive(pid: u32) -> bool {
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .expect("tasklist");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|line| line.starts_with('"'))
}

fn assert_gone_within(pid: u32, what: &str, budget: Duration) {
    let deadline = Instant::now() + budget;
    while alive(pid) {
        assert!(
            Instant::now() < deadline,
            "{what} (pid {pid}) still runs after {budget:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn closing_a_block_ends_every_program_on_its_console(host: ConsoleHost) {
    let path = start_server_on(host);
    let mut a = Client::connect(&path);
    let block = block_for(&mut a);
    let shell = shell_pid(&mut a, block);
    a.input(block, PING);
    let ping = a.wait_for_number(block, "PING=");
    assert!(alive(shell) && alive(ping), "shell {shell}, ping {ping}");

    a.request(|req| Message::CloseBlock { req, block });
    assert_gone_within(shell, "the shell", Duration::from_secs(2));
    // GRACE, Windows' 5 s console-close timeout, and a margin.
    assert_gone_within(ping, "the ping", Duration::from_secs(7));
}

fn closing_a_flooding_block_leaves_the_server_responsive(host: ConsoleHost) {
    let path = start_server_on(host);
    let mut a = Client::connect(&path);
    a.parse = false;
    let flooding = block_for(&mut a);
    let shell = shell_pid(&mut a, flooding);
    // The echo keeps the caret; the output does not.
    a.input(flooding, b"for /l %i in () do @echo fl^ood\r");
    a.wait_for(flooding, "flood");
    a.request(|req| Message::Detach { req });

    let mut b = Client::connect(&path);
    let other = block_for(&mut b);
    run_and_wait(&mut b, other, 51);

    a.request(|req| Message::CloseBlock {
        req,
        block: flooding,
    });
    let started = Instant::now();
    let marker = run_and_wait(&mut b, other, 52);
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "{marker} took {elapsed:?} while a flooding block closed"
    );
    assert_gone_within(shell, "the flooding shell", Duration::from_secs(2));
    b.request(|req| Message::CloseBlock { req, block: other });
}

fn a_killed_server_takes_its_blocks_with_it(host: ConsoleHost) {
    let name = format!("rusty-mux-killed-{}-{host:?}.sock", std::process::id());
    let path = std::env::temp_dir().join(name);
    let _ = std::fs::remove_file(&path);
    let mut server = KillOnDrop(
        Command::new(env!("CARGO_BIN_EXE_rusty-mux-server"))
            .arg(&path)
            .env(ConsoleHost::ENV, console_host_name(host))
            .stdin(Stdio::null())
            .spawn()
            .expect("start the server"),
    );
    wait_until_listening(&path);

    let mut a = Client::connect(&path);
    let block = block_for(&mut a);
    let shell = shell_pid(&mut a, block);
    assert!(alive(shell), "shell {shell}");

    server.0.kill().expect("kill the server");
    server.0.wait().expect("reap the server");
    assert_gone_within(shell, "the dead server's shell", Duration::from_secs(7));
    let _ = std::fs::remove_file(&path);
}

/// A server process that a failing test still ends.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // Both fail harmlessly once the test has killed and reaped it.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_until_listening(path: &Path) {
    let deadline = Instant::now() + PATIENCE;
    while connect(path).is_err() {
        assert!(Instant::now() < deadline, "server never listened");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn a_program_that_exits_reports_its_code(host: ConsoleHost) {
    let path = start_server_on(host);
    let wanted = Features::PROGRAM_EVENTS.union(Features::CHECKPOINT_V6);
    let mut a = Client::connect_with(&path, wanted, None);
    let block = block_for(&mut a);
    a.input(block, b"exit 3\r");
    let closed = Event::BlockClosed { block };
    while !a.events.iter().any(|(_, e)| *e == closed) {
        a.recv();
    }
    let exited = Event::ProgramExited {
        block,
        status: ExitStatus::Code(3),
    };
    let at = |event: &Event| a.events.iter().position(|(_, e)| e == event);
    let exited_at = at(&exited).unwrap_or_else(|| panic!("no exit in {:?}", a.events));
    assert!(Some(exited_at) < at(&closed), "exit after the close");
}

/// Windows has no foreground process group, so `terminal.process` lists
/// only the block's own program, by pid alone.
fn terminal_process_names_the_programs_pid(host: ConsoleHost) {
    let path = start_server_on(host);
    let wanted = Features::CHECKPOINT_V6.union(Features::BLOCK_CALL);
    let mut a = Client::connect_with(&path, wanted, None);
    let block = block_for(&mut a);
    let shell = shell_pid(&mut a, block);
    let answer = a.answer(|req| Message::BlockCall {
        req,
        block,
        method: "terminal.process".into(),
        args: Vec::<(String, Value)>::new(),
    });
    let [Message::Text { text, .. }] = answer.as_slice() else {
        panic!("expected Text, got {answer:?}")
    };
    assert_eq!(text.as_str(), format!("{shell}\t\t"), "{text:?}");
}
