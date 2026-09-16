#![cfg(windows)]
// `process`'s Windows arm -- see `src/process/mod.rs`'s docs and
// `docs/decision-request-windows-process-signal-ipc.md` for the design
// (spawn/wait/kill match Unix exactly via portable `std::process::Child`
// methods; piped stdio is `spawn_blocking`-backed here rather than
// reactor-driven, Decision 1). No `/bin/sh`/`cat` on Windows -- `cmd.exe
// /C` plus built-in commands (`echo`, `exit`, `sort` as a stdin-echo
// stand-in, `ping` as a sleep stand-in with no reliance on a real
// console the way `timeout.exe` needs) stand in for this file's Unix
// counterpart (`tests/process.rs`, which stays `#![cfg(unix)]`-only).
// `arg0`/`process_group` aren't exercised here -- both are `#[cfg(unix)]`-only
// (see `Command`'s own docs for why).

use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt};
use rusty_tokio::process::{Command, Stdio};
use rusty_tokio::Runtime;
use std::time::Duration;

fn cmd() -> Command {
    let mut c = Command::new("cmd.exe");
    c.arg("/C");
    c
}

#[test]
fn spawn_and_wait_reports_the_exit_code() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut child = cmd().arg("exit 3").spawn().unwrap();
        let status = child.wait().await.unwrap();
        assert_eq!(status.code(), Some(3));
    });
}

#[test]
fn piped_stdout_is_read_asynchronously() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut child = cmd()
            .arg("echo hello from child")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let mut stdout = child.stdout.take().unwrap();
        let mut contents = Vec::new();
        stdout.read_to_end(&mut contents).await.unwrap();
        assert_eq!(
            String::from_utf8_lossy(&contents).trim_end(),
            "hello from child"
        );

        let status = child.wait().await.unwrap();
        assert!(status.success());
    });
}

#[test]
fn piped_stdin_is_written_and_sort_echoes_it_back() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        // `sort` (a Windows console built-in, no external dependency)
        // reads all of stdin, then writes it back out once it sees
        // EOF -- for a single line of input, "sorted" is a no-op, so
        // this is the Windows stand-in for Unix's `cat` echo test.
        let mut child = cmd()
            .arg("sort")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();

        stdin.write_all(b"round trip through sort").await.unwrap();
        // `sort` keeps buffering until it sees EOF on stdin -- dropping
        // our write end delivers that.
        drop(stdin);

        let mut contents = Vec::new();
        stdout.read_to_end(&mut contents).await.unwrap();
        assert_eq!(
            String::from_utf8_lossy(&contents).trim_end(),
            "round trip through sort"
        );

        let status = child.wait().await.unwrap();
        assert!(status.success());
    });
}

#[test]
fn stderr_is_captured_separately_from_stdout() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut child = cmd()
            .arg("echo to-stdout & echo to-stderr 1>&2")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();

        let mut out = Vec::new();
        stdout.read_to_end(&mut out).await.unwrap();
        let mut err = Vec::new();
        stderr.read_to_end(&mut err).await.unwrap();

        assert_eq!(String::from_utf8_lossy(&out).trim_end(), "to-stdout");
        assert_eq!(String::from_utf8_lossy(&err).trim_end(), "to-stderr");

        child.wait().await.unwrap();
    });
}

#[test]
fn try_wait_reports_none_while_running_then_some_after_exit() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        // `ping -n 2 127.0.0.1` takes ~1s (one interval between the two
        // echo requests) -- the standard Windows-console "sleep briefly"
        // idiom, since there's no built-in `sleep`/`timeout.exe` that
        // reliably works without a real interactive console attached.
        let mut child = cmd().arg("ping -n 2 127.0.0.1 >NUL").spawn().unwrap();

        assert!(child.try_wait().unwrap().is_none());

        let status = child.wait().await.unwrap();
        assert!(status.success());

        // Already reaped -- try_wait keeps reporting the same status
        // rather than erroring on an already-waited-for child.
        assert_eq!(child.try_wait().unwrap().unwrap().code(), status.code());
    });
}

#[test]
fn kill_terminates_a_running_child() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut child = cmd().arg("ping -n 31 127.0.0.1 >NUL").spawn().unwrap();

        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();

        let status = rusty_tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("a killed child should exit promptly")
            .unwrap();
        assert!(!status.success());

        // Killing an already-reaped child is a no-op, not an error.
        child.kill().unwrap();
    });
}

#[test]
fn id_stays_available_after_the_child_has_been_waited_on() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let mut child = cmd().arg("exit 0").spawn().unwrap();
        let id = child.id();
        assert!(id > 0);
        child.wait().await.unwrap();
        assert_eq!(child.id(), id);
    });
}

#[test]
fn kill_on_drop_kills_a_still_running_child_when_dropped() {
    let rt = Runtime::new().unwrap();
    let id = rt.block_on(async {
        let mut command = cmd();
        command.arg("ping -n 31 127.0.0.1 >NUL").kill_on_drop(true);
        let child = command.spawn().unwrap();
        let id = child.id();
        drop(child);
        id
    });
    // Give the best-effort background reap a moment, then confirm the
    // process is actually gone -- `tasklist` is a Windows built-in,
    // no external dependency, the closest analog to the Unix test's
    // own `kill(pid, 0)`-based liveness probe.
    std::thread::sleep(Duration::from_millis(500));
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {id}")])
        .output()
        .unwrap();
    let listing = String::from_utf8_lossy(&output.stdout);
    assert!(
        !listing.contains(&id.to_string()),
        "child pid {id} should have been killed on drop, tasklist still shows it: {listing}"
    );
}

#[test]
fn status_spawns_and_waits_reporting_the_exit_code() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let status = cmd().arg("exit 7").status().await.unwrap();
        assert_eq!(status.code(), Some(7));
    });
}

#[test]
fn output_captures_stdout_and_stderr_and_reports_the_exit_code() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let output = cmd()
            .arg("echo to-stdout & echo to-stderr 1>&2 & exit 2")
            .output()
            .await
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            "to-stdout"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim_end(),
            "to-stderr"
        );
    });
}

#[test]
fn wait_with_output_works_when_only_stdout_is_piped() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let child = cmd()
            .arg("echo only stdout")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.wait_with_output().await.unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            "only stdout"
        );
        assert!(output.stderr.is_empty());
    });
}

#[test]
fn child_stdout_reads_a_large_write_across_multiple_polls() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        // Bigger than `process::CHUNK_CAP` (64 KiB) so this exercises
        // more than one blocking-pool round trip through `ReadState`.
        let mut child = cmd()
            .arg("for /L %i in (1,1,20000) do @echo line%i")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut contents = Vec::new();
        stdout.read_to_end(&mut contents).await.unwrap();
        assert!(contents.len() > 64 * 1024);
        assert!(String::from_utf8_lossy(&contents).contains("line20000"));
        child.wait().await.unwrap();
    });
}
