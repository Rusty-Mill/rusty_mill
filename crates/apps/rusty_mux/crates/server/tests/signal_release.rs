//! Signal and release (C76): the owner of a block signals its program's
//! group, or closes its PTY and waits without the mux ever killing the
//! program. Linux only, as the feature is.

#![cfg(target_os = "linux")]

mod common;

use std::time::{Duration, Instant};

use common::{Client, PATIENCE, start_server, window_for};
use rusty_mux_core::{BlockId, Size};
use rusty_mux_proto::{BlockSignal, Entry, Features, Launch, Message, Program};

const OWNER: Features = Features::CHECKPOINT_V6
    .union(Features::LAUNCH)
    .union(Features::INSPECT)
    .union(Features::OWNERSHIP)
    .union(Features::HOLD)
    .union(Features::SIGNAL_RELEASE);

fn owned(script: &str, hold: bool) -> Launch {
    Launch {
        program: Some(Program {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
        }),
        own: true,
        hold,
        ..Launch::default()
    }
}

/// An owned block running `script`, attached, once it has printed `ready`.
fn launch(client: &mut Client, script: &str, hold: bool) -> BlockId {
    let window = window_for(client);
    let size = Size::new(80, 24).expect("size");
    let id = client.request(|req| Message::CreateBlock {
        req,
        window,
        size,
        launch: owned(script, hold),
    });
    let block = BlockId::from_uuid(id.expect("a block id"));
    client.request(|req| Message::Attach { req, block });
    client.wait_ready(block);
    screen_until(client, block, "ready");
    block
}

/// The block's screen once it shows `needle`, which may have come before
/// the attach's checkpoint or after it.
fn screen_until(client: &mut Client, block: BlockId, needle: &str) -> String {
    loop {
        let text = client.text(block);
        if text.contains(needle) {
            return text;
        }
        client.recv();
    }
}

/// The number the screen shows after `PID=`.
fn pid(client: &mut Client, block: BlockId) -> u32 {
    let text = screen_until(client, block, "PID=");
    let digits: String = text
        .split("PID=")
        .nth(1)
        .unwrap_or_default()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().expect("a pid on screen")
}

fn refused(client: &mut Client, make: impl FnOnce(u64) -> Message) -> String {
    match client.answer(make).as_slice() {
        [Message::Failed { reason, .. }] => reason.clone(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The block's listing entry's exited mark; `None` once it is gone.
fn exited(client: &mut Client, block: BlockId) -> Option<bool> {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: Entry::Block {
                    block: b, exited, ..
                },
                ..
            } if b == block => Some(exited),
            _ => None,
        })
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done() {
        assert!(Instant::now() < deadline, "never {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether `pid` is a live process, not a zombie or gone.
fn alive(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    // The state follows the command's closing parenthesis.
    let state = stat
        .rsplit(')')
        .next()
        .and_then(|rest| rest.trim().chars().next());
    !matches!(state, Some('Z' | 'X') | None)
}

fn kill(pid: u32) {
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .expect("run kill");
    assert!(status.success(), "kill -9 {pid}");
}

/// Kills the group `pid` leads, the block program's, children included.
fn kill_group(pid: u32) {
    let status = std::process::Command::new("kill")
        .args(["-9", "--", &format!("-{pid}")])
        .status()
        .expect("run kill");
    assert!(status.success(), "kill -9 -- -{pid}");
}

#[test]
fn term_reaches_a_trap_in_the_programs_group() {
    let socket = start_server();
    let mut owner = Client::connect_with(&socket, OWNER, None);
    let script = "trap 'echo TRAPPED-TERM; exit 0' TERM; echo ready; while :; do sleep 0.1; done";
    let block = launch(&mut owner, script, false);

    owner.request(|req| Message::SignalBlock {
        req,
        block,
        signal: BlockSignal::Term,
    });
    screen_until(&mut owner, block, "TRAPPED-TERM");
}

#[test]
fn a_released_program_that_ignores_hangups_is_never_killed() {
    let socket = start_server();
    let mut owner = Client::connect_with(&socket, OWNER, None);
    let script = "trap '' HUP; echo PID=$$; echo ready; while :; do sleep 0.1; done";
    let block = launch(&mut owner, script, false);
    let pid = pid(&mut owner, block);
    // Held from before, so only the release refuses the input below.
    owner.request(|req| Message::AcquireLease { req, block });

    owner.request(|req| Message::ReleaseBlock { req, block });
    std::thread::sleep(Duration::from_secs(2));
    assert!(alive(pid), "pid {pid} was killed after the release");
    assert_eq!(exited(&mut owner, block), Some(true), "read-only meanwhile");
    let typed = refused(&mut owner, |req| Message::Input {
        req,
        block,
        data: b"x".to_vec(),
    });
    assert!(typed.contains("its pty was released"), "{typed}");
    let again = refused(&mut owner, |req| Message::SignalBlock {
        req,
        block,
        signal: BlockSignal::Term,
    });
    assert!(again.starts_with("exited"), "{again}");

    // Its owner ends it; an unheld block then closes.
    kill(pid);
    wait_until("closed", || exited(&mut owner, block).is_none());
}

#[test]
fn a_held_block_stays_after_its_released_program_exits() {
    let socket = start_server();
    let mut owner = Client::connect_with(&socket, OWNER, None);
    let script = "trap '' HUP; echo PID=$$; echo ready HELD-SCREEN; while :; do sleep 0.1; done";
    let block = launch(&mut owner, script, true);
    let pid = pid(&mut owner, block);

    owner.request(|req| Message::ReleaseBlock { req, block });
    kill(pid);
    wait_until("reaped", || !alive(pid));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(exited(&mut owner, block), Some(true), "still listed");
    owner.request(|req| Message::CloseBlock { req, block });
    assert_eq!(exited(&mut owner, block), None, "closed");
}

/// A write already under way when the owner releases gives the master up:
/// the program's terminal hangs up, and no byte reaches it afterwards.
#[test]
fn a_release_abandons_a_write_already_under_way() {
    const SENT: usize = 1 << 20;
    let socket = start_server();
    let mut owner = Client::connect_with(&socket, OWNER, None);
    let mut writer = Client::connect_with(&socket, OWNER, None);
    let dir = std::env::temp_dir().join(format!("signal-release-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let out = dir.join("read");
    let _ = std::fs::remove_file(&out);
    // Raw, so the terminal neither echoes nor waits for a line; the
    // program reads nothing until well after the release.
    let script = format!(
        "stty raw -echo; trap '' HUP; echo PID=$$; echo ready; sleep 1; \
         head -c {SENT} | wc -c > {out}.part; mv {out}.part {out}; sleep 30",
        out = out.display()
    );
    let block = launch(&mut owner, &script, false);
    let pid = pid(&mut owner, block);
    writer.request(|req| Message::Attach { req, block });
    writer.request(|req| Message::AcquireLease { req, block });
    // Far more than the terminal buffers, so the write stays pending.
    writer.input(block, &vec![b'x'; SENT]);
    std::thread::sleep(Duration::from_millis(200));

    owner.request(|req| Message::ReleaseBlock { req, block });
    wait_until("read to its end", || out.exists());
    let read: usize = std::fs::read_to_string(&out)
        .expect("count")
        .trim()
        .parse()
        .expect("a byte count");
    assert!(
        read < SENT,
        "{read} of {SENT} bytes arrived after the release"
    );
    assert_eq!(
        exited(&mut writer, block),
        Some(true),
        "the writer still served"
    );
    assert!(alive(pid), "pid {pid} was killed after the release");
    // Its `sleep 30` too, not only the shell.
    kill_group(pid);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_the_owner_may_signal_or_release() {
    let socket = start_server();
    let mut owner = Client::connect_with(&socket, OWNER, None);
    let mut peer = Client::connect_with(&socket, OWNER, None);
    let block = launch(&mut owner, "echo ready; sleep 30", false);

    let signal = refused(&mut peer, |req| Message::SignalBlock {
        req,
        block,
        signal: BlockSignal::Kill,
    });
    let release = refused(&mut peer, |req| Message::ReleaseBlock { req, block });
    assert!(signal.contains("own"), "{signal}");
    assert!(release.contains("own"), "{release}");
    assert_eq!(exited(&mut owner, block), Some(false), "untouched");
}

/// A block made without `own` has no owner, so no client may signal or
/// release it, its creator included (D230): it behaves as before C76.
#[test]
fn an_unowned_block_is_never_signalled_or_released() {
    let socket = start_server();
    let mut maker = Client::connect_with(&socket, OWNER, None);
    let window = window_for(&mut maker);
    let size = Size::new(80, 24).expect("size");
    let launch = Launch {
        own: false,
        ..owned("echo PID=$$; echo ready; sleep 30", false)
    };
    let id = maker.request(|req| Message::CreateBlock {
        req,
        window,
        size,
        launch,
    });
    let block = BlockId::from_uuid(id.expect("a block id"));
    maker.request(|req| Message::Attach { req, block });
    maker.wait_ready(block);
    let pid = pid(&mut maker, block);

    let signal = refused(&mut maker, |req| Message::SignalBlock {
        req,
        block,
        signal: BlockSignal::Kill,
    });
    let release = refused(&mut maker, |req| Message::ReleaseBlock { req, block });
    assert!(signal.contains("no owner"), "{signal}");
    assert!(release.contains("no owner"), "{release}");
    assert!(alive(pid), "pid {pid} was signalled");
    assert_eq!(exited(&mut maker, block), Some(false), "untouched");
    kill_group(pid);
}

#[test]
fn signal_and_release_need_the_feature() {
    let socket = start_server();
    let mut owner = Client::connect_with(&socket, OWNER, None);
    let block = launch(&mut owner, "echo ready; sleep 30", false);
    let plain = OWNER.without(Features::SIGNAL_RELEASE);
    let mut older = Client::connect_with(&socket, plain, None);
    let release = refused(&mut older, |req| Message::ReleaseBlock { req, block });
    assert!(release.contains("requires feature bits"), "{release}");
    let signal = refused(&mut older, |req| Message::SignalBlock {
        req,
        block,
        signal: BlockSignal::Kill,
    });
    assert!(signal.contains("requires feature bits"), "{signal}");
    assert_eq!(exited(&mut owner, block), Some(false), "untouched");
}
