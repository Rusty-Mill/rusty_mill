//! Live ConDrv tests (#1222, PR 1): a real console server, real children
//! attached to it, and a minimal in-test host that answers their packets.
//! No conhost is involved. Windows only; like `tests/pty.rs`, nothing
//! here runs on the Linux CI leg.
//!
//! **Every blocking call is bounded.** Each `wait` takes an absolute
//! deadline derived from [`BUDGET`], so a hang fails the test that hit it
//! with a message, instead of stalling the job.
//!
//! The mini host answers CONNECT, CREATE_OBJECT, CLOSE_OBJECT, DISCONNECT
//! and RAW_WRITE as conhost does, and every USER_DEFINED API with
//! `STATUS_NOT_IMPLEMENTED`. That is enough for the clients used here
//! (`cmd /c`, `ping`) to start, print and exit; the real host is
//! `rusty_conhost`.

#![cfg(windows)]
#![allow(unsafe_code)] // accept_ctrl_c below: one raw SetConsoleCtrlHandler call, its own SAFETY comment.

use std::time::{Duration, Instant};

use platform::error::ErrorKind;
use std::os::windows::process::CommandExt;

use platform::process::{Child, Command, ExitStatus, GroupSpec, Signal};
use platform_windows::condrv::{
    self, Attached, ConsoleServer, CtrlEvent, IoComplete, IoDescriptor, Ready, Waker,
};

const BUDGET: Duration = Duration::from_secs(15);

const CONNECT: u32 = 1;
const DISCONNECT: u32 = 2;
const CREATE_OBJECT: u32 = 3;
const RAW_WRITE: u32 = 5;
const USER_DEFINED: u32 = 7;

const STATUS_SUCCESS: i32 = 0;
const STATUS_NOT_IMPLEMENTED: i32 = 0xC000_0002_u32 as i32;
const STATUS_UNSUCCESSFUL: i32 = 0xC000_0001_u32 as i32;

/// Serializes the tests: each attaches a child to a console of its own,
/// and `cargo test` would otherwise run them at once. The spawn lock
/// already serializes the spawns themselves; this keeps the timing
/// assertions free of a neighbour's load.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn command(program: &str, args: &[&str]) -> Command {
    let cwd = std::env::current_dir().expect("current_dir");
    let mut cmd = Command::new(program, cwd);
    cmd.argv = args.iter().map(Into::into).collect();
    cmd
}

/// What the mini host saw.
#[derive(Default)]
struct Seen {
    functions: Vec<u32>,
    connect_pids: Vec<usize>,
    /// RAW_WRITE bytes and USER_DEFINED payloads, as received.
    payload: Vec<u8>,
}

impl Seen {
    fn text_contains(&self, needle: &str) -> bool {
        let utf8 = needle.as_bytes();
        let utf16: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
        self.payload.windows(utf8.len()).any(|w| w == utf8)
            || self
                .payload
                .windows(utf16.len())
                .any(|w| w == utf16.as_slice())
    }
}

/// `CD_CONNECTION_INFORMATION { Process, Input, Output }`.
fn connection_information() -> Vec<u8> {
    [0x51_usize, 0x52, 0x53]
        .iter()
        .flat_map(|v| v.to_ne_bytes())
        .collect()
}

fn read_all(server: &mut ConsoleServer, d: &IoDescriptor) -> Vec<u8> {
    let mut buf = vec![0u8; d.input_size as usize];
    let n = server.read_input(d.id, 0, &mut buf).expect("read_input");
    assert_eq!(n, buf.len(), "read_input returned a short payload");
    buf
}

/// Answer one packet, as the reply for the next READ_IO.
fn answer(
    server: &mut ConsoleServer,
    d: &IoDescriptor,
    seen: &mut Seen,
    next_object: &mut usize,
) -> (i32, usize, Vec<u8>) {
    seen.functions.push(d.function);
    match d.function {
        CONNECT => {
            seen.connect_pids.push(d.process);
            let payload = read_all(server, d);
            assert!(!payload.is_empty(), "CONNECT carries CONSOLE_SERVER_MSG");
            let info = connection_information();
            (STATUS_SUCCESS, info.len(), info)
        }
        CREATE_OBJECT => {
            *next_object += 1;
            (STATUS_SUCCESS, *next_object, Vec::new())
        }
        RAW_WRITE => {
            seen.payload.extend(read_all(server, d));
            (STATUS_SUCCESS, d.input_size as usize, Vec::new())
        }
        USER_DEFINED => {
            seen.payload.extend(read_all(server, d));
            (STATUS_NOT_IMPLEMENTED, 0, Vec::new())
        }
        2 | 4 | 8 => (STATUS_SUCCESS, 0, Vec::new()),
        _ => (STATUS_UNSUCCESSFUL, 0, Vec::new()),
    }
}

/// Serve `attached` until its child exits and disconnects, or [`BUDGET`]
/// runs out. `tick` runs after every wait, with the child's pid.
fn serve(
    attached: &mut Attached,
    waker: &Waker,
    tick: impl FnMut(&Seen, u32),
) -> (Seen, Option<ExitStatus>) {
    serve_for(attached, waker, BUDGET, tick)
}

/// [`serve`] with a budget of `budget`.
fn serve_for(
    attached: &mut Attached,
    waker: &Waker,
    budget: Duration,
    mut tick: impl FnMut(&Seen, u32),
) -> (Seen, Option<ExitStatus>) {
    let deadline = Instant::now() + budget;
    let pid = attached.child.id();
    let mut seen = Seen::default();
    let mut next_object = 0x100;
    let mut reply: Option<(IoDescriptor, i32, usize, Vec<u8>)> = None;
    let mut exited = false;
    let mut read_pending = false;
    while Instant::now() < deadline {
        if !read_pending {
            let complete = reply
                .as_ref()
                .map(|(d, status, information, write_back)| IoComplete {
                    id: d.id,
                    status: *status,
                    information: *information,
                    write_back,
                });
            match attached.server.submit_read(complete.as_ref()) {
                Ok(()) => read_pending = true,
                Err(e) if e.kind == ErrorKind::BrokenPipe => break,
                Err(e) => panic!("submit_read: {e:?}"),
            }
            reply = None;
        }
        let exit = (!exited).then_some(&attached.wait);
        let ready: Ready = match attached.server.wait(waker, exit, Some(deadline)) {
            Ok(r) => r,
            Err(e) if e.kind == ErrorKind::BrokenPipe => break,
            Err(e) => panic!("wait: {e:?}"),
        };
        exited |= ready.exited;
        if let Some(d) = ready.io {
            read_pending = false;
            let (status, information, write_back) =
                answer(&mut attached.server, &d, &mut seen, &mut next_object);
            reply = Some((d, status, information, write_back));
        }
        tick(&seen, pid);
        if exited && seen.functions.contains(&DISCONNECT) {
            // Nothing more will arrive: complete the last answer on its own
            // rather than with a READ_IO that would wait for the budget.
            if let Some((d, status, information, write_back)) = reply.take() {
                attached
                    .server
                    .complete_io(&IoComplete {
                        id: d.id,
                        status,
                        information,
                        write_back: &write_back,
                    })
                    .expect("complete_io");
            }
            break;
        }
    }
    let status = attached.child.try_wait().expect("try_wait");
    (seen, status)
}

#[test]
fn probe_finds_condrv_and_console_control() {
    condrv::probe().expect("probe: ConDrv and ConsoleControl on this build");
}

#[test]
fn join_group_is_invalid_input() {
    let mut cmd = command("cmd.exe", &["/c", "exit", "0"]);
    cmd.group = GroupSpec::JoinGroup(1);
    let Err(e) = condrv::spawn_attached(&cmd) else {
        panic!("JoinGroup was accepted");
    };
    assert_eq!(e.kind, ErrorKind::InvalidInput);
}

#[test]
fn connect_disconnect_round_trip_and_exit_code() {
    let _serial = lock();
    let mut attached = condrv::spawn_attached(&command("cmd.exe", &["/c", "echo", "hi&exit", "7"]))
        .expect("spawn_attached");
    let waker = Waker::new().expect("Waker::new");
    let (seen, status) = serve(&mut attached, &waker, |_, _| {});

    assert_eq!(
        seen.functions.first(),
        Some(&CONNECT),
        "CONNECT comes first: {:?}",
        seen.functions
    );
    assert!(
        seen.functions.contains(&DISCONNECT),
        "no DISCONNECT: {:?}",
        seen.functions
    );
    assert_eq!(
        seen.connect_pids.first().copied(),
        Some(attached.child.id() as usize)
    );
    assert!(seen.text_contains("hi"), "the child's output never arrived");
    assert_eq!(status, Some(ExitStatus::Code(7)));
}

/// A client is opened only through the thread its CONNECT came from
/// (#1259): a pid that thread doesn't belong to, as a reused pid would
/// be, is refused and that process is never touched.
#[test]
fn a_client_is_opened_only_through_its_connect_thread() {
    let _serial = lock();
    let mut attached = condrv::spawn_attached(&command("ping.exe", &["-n", "30", "127.0.0.1"]))
        .expect("spawn_attached");
    let waker = Waker::new().expect("Waker::new");
    attached.server.submit_read(None).expect("submit_read");
    let deadline = Instant::now() + BUDGET;
    let connect = loop {
        let ready = attached
            .server
            .wait(&waker, None, Some(deadline))
            .expect("wait");
        if let Some(d) = ready.io {
            break d;
        }
        assert!(Instant::now() < deadline, "no CONNECT");
    };
    assert_eq!(connect.function, CONNECT);
    let (pid, tid) = (connect.process as u32, connect.object as u32);
    assert_eq!(pid, attached.child.id());

    let mut other = std::process::Command::new("ping.exe")
        .args(["-n", "30", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("spawn another process");
    let refused = condrv::ClientProcess::open(other.id(), tid).err();
    assert_eq!(refused.map(|e| e.kind), Some(ErrorKind::InvalidInput));
    let gone = condrv::ClientProcess::open(pid, u32::MAX - 3).err();
    assert!(gone.is_some(), "a thread that doesn't exist is refused");
    assert!(
        matches!(other.try_wait(), Ok(None)),
        "the other process runs on"
    );

    let client = condrv::ClientProcess::open(pid, tid).expect("the CONNECT's client");
    client.terminate().expect("terminate");
    let status = Box::new(attached.child).wait().expect("wait");
    assert_eq!(status, ExitStatus::Code(0xC000_013A_u32 as i32));
    let _ = other.kill();
    let _ = other.wait();
}

/// What ConDrv does with a CONNECT whose client dies before the host
/// reads it (#1259, design §7.3): it must not be delivered, or a host
/// could not tell the client's thread id from a reused one. Probe for
/// the identity check's generation argument; run on Windows.
#[test]
fn a_connect_whose_client_died_unread_is_not_delivered() {
    let _serial = lock();
    let mut attached = condrv::spawn_attached(&command("ping.exe", &["-n", "30", "127.0.0.1"]))
        .expect("spawn_attached");
    // The child blocks in its CONNECT; nothing reads it yet.
    std::thread::sleep(Duration::from_millis(500));
    let pid = attached.child.id();
    attached.child.kill_single(Signal::Kill).expect("kill");
    let status = Box::new(attached.child).wait().expect("wait");
    assert!(matches!(status, ExitStatus::Code(_)), "{status:?}");
    let waker = Waker::new().expect("Waker::new");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        match attached.server.submit_read(None) {
            Ok(()) => {}
            Err(e) if e.kind == ErrorKind::BrokenPipe => break,
            Err(e) => panic!("submit_read: {e:?}"),
        }
        match attached.server.wait(&waker, None, Some(deadline)) {
            Ok(Ready { io: Some(d), .. }) => {
                seen.push((d.function, d.process, d.object));
                if d.function == CONNECT {
                    break;
                }
                // Answer it so the next read can be issued.
                attached
                    .server
                    .complete_io(&IoComplete {
                        id: d.id,
                        status: STATUS_SUCCESS,
                        information: 0,
                        write_back: &[],
                    })
                    .expect("complete_io");
            }
            Ok(_) => break,
            Err(e) if e.kind == ErrorKind::BrokenPipe => break,
            Err(e) => panic!("wait: {e:?}"),
        }
    }
    assert!(
        !seen.iter().any(|&(f, _, _)| f == CONNECT),
        "a dead client's CONNECT (pid {pid}) was delivered: {seen:?}"
    );
}

/// What completing a CONNECT does once its client has died after the
/// host read it (#1259): a probe for whether a successful reply proves the
/// client's thread was alive when the host opened it. Run on Windows.
#[test]
fn a_connect_reply_to_a_client_that_died_after_the_read_fails() {
    let _serial = lock();
    let mut attached = condrv::spawn_attached(&command("ping.exe", &["-n", "30", "127.0.0.1"]))
        .expect("spawn_attached");
    let waker = Waker::new().expect("Waker::new");
    attached.server.submit_read(None).expect("submit_read");
    let deadline = Instant::now() + BUDGET;
    let connect = loop {
        let ready = attached
            .server
            .wait(&waker, None, Some(deadline))
            .expect("wait");
        if let Some(d) = ready.io {
            break d;
        }
        assert!(Instant::now() < deadline, "no CONNECT");
    };
    assert_eq!(connect.function, CONNECT);
    let _payload = read_all(&mut attached.server, &connect);
    attached.child.kill_single(Signal::Kill).expect("kill");
    let _ = Box::new(attached.child).wait().expect("wait");
    std::thread::sleep(Duration::from_millis(200));
    let info = connection_information();
    let done = attached.server.complete_io(&IoComplete {
        id: connect.id,
        status: STATUS_SUCCESS,
        information: info.len(),
        write_back: &info,
    });
    let err = done.err().expect("the reply to a dead client fails");
    eprintln!("dead-client CONNECT reply: {err:?}");
}

#[test]
fn packet_bounds_and_completion_are_checked() {
    let _serial = lock();
    let mut attached =
        condrv::spawn_attached(&command("cmd.exe", &["/c", "exit", "0"])).expect("spawn_attached");
    let waker = Waker::new().expect("Waker::new");
    let server = &mut attached.server;

    server.submit_read(None).expect("submit_read");
    let again = server
        .submit_read(None)
        .expect_err("a second read was accepted");
    assert_eq!(again.kind, ErrorKind::WouldBlock);

    let ready = server
        .wait(&waker, None, Some(Instant::now() + BUDGET))
        .expect("wait");
    let d = ready.io.expect("CONNECT within the budget");
    assert_eq!(d.function, CONNECT);

    let mut byte = [0u8; 1];
    let past = server
        .read_input(d.id, d.input_size + 1, &mut byte)
        .expect_err("read past input");
    assert_eq!(past.kind, ErrorKind::InvalidInput);
    let past = server
        .write_output(d.id, d.output_size, &[0])
        .expect_err("write past output");
    assert_eq!(past.kind, ErrorKind::InvalidInput);
    let too_big = vec![0u8; d.output_size as usize + 1];
    let e = server
        .complete_io(&IoComplete {
            id: d.id,
            status: 0,
            information: 0,
            write_back: &too_big,
        })
        .expect_err("oversized write_back");
    assert_eq!(e.kind, ErrorKind::InvalidInput);

    let info = connection_information();
    server
        .complete_io(&IoComplete {
            id: d.id,
            status: STATUS_SUCCESS,
            information: info.len(),
            write_back: &info,
        })
        .expect("complete_io CONNECT");
    let gone = server
        .read_input(d.id, 0, &mut byte)
        .expect_err("read after completion");
    assert_eq!(gone.kind, ErrorKind::NotFound);
    attached.server.close();
}

#[test]
fn deadline_waker_and_cancellation() {
    let _serial = lock();
    let mut attached =
        condrv::spawn_attached(&command("cmd.exe", &["/c", "exit", "0"])).expect("spawn_attached");
    let waker = std::sync::Arc::new(Waker::new().expect("Waker::new"));
    let (seen, _) = serve(&mut attached, &waker, |_, _| {});
    assert!(seen.functions.contains(&DISCONNECT));
    let server = &mut attached.server;

    // Nothing will arrive now: the read stays pending. `serve` may have
    // left one in flight already.
    match server.submit_read(None) {
        Ok(()) => {}
        Err(e) => assert_eq!(e.kind, ErrorKind::WouldBlock, "{e:?}"),
    }

    // A past deadline is a poll.
    let started = Instant::now();
    let ready = server.wait(&waker, None, Some(started)).expect("poll");
    assert_eq!(
        ready,
        Ready {
            io: None,
            exited: false
        }
    );
    assert!(started.elapsed() < Duration::from_secs(1));

    // The deadline is absolute.
    let deadline = Instant::now() + Duration::from_millis(300);
    let ready = server
        .wait(&waker, None, Some(deadline))
        .expect("timed wait");
    assert_eq!(
        ready,
        Ready {
            io: None,
            exited: false
        }
    );
    assert!(
        Instant::now() >= deadline,
        "the wait ended before its deadline"
    );

    // A wake from another thread ends an unbounded wait.
    let remote = std::sync::Arc::clone(&waker);
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        remote.wake();
    });
    let ready = server
        .wait(&waker, None, Some(Instant::now() + BUDGET))
        .expect("woken wait");
    assert_eq!(
        ready,
        Ready {
            io: None,
            exited: false
        }
    );
    t.join().expect("waker thread");

    // Cancelling is not a wait; the completion arrives through `wait`.
    server.cancel_read();
    let outcome = server.wait(&waker, None, Some(Instant::now() + BUDGET));
    match outcome {
        Err(e) => assert_eq!(e.kind, ErrorKind::Interrupted, "{e:?}"),
        Ok(r) => assert!(r.io.is_some(), "the cancelled read never completed"),
    }
    // A cancel with no read in flight is a no-op.
    server.cancel_read();
    attached.server.close();
}

/// Clear this test process's "ignore Ctrl+C" flag. nextest starts each
/// test in a new process group, which sets it, and children inherit it;
/// `spawn_attached` deliberately leaves the caller's policy alone, so a
/// caller that wants its children to see Ctrl+C opts in itself.
fn accept_ctrl_c() {
    // SAFETY: a null handler only sets this process's Ctrl+C flag.
    let ok = unsafe { windows_sys::Win32::System::Console::SetConsoleCtrlHandler(None, 0) };
    assert_ne!(ok, 0, "SetConsoleCtrlHandler(NULL, FALSE)");
}

/// Attach `ping -n 30`, send it Ctrl+C after 1.5 s, and serve it for
/// `budget`. Returns its status when serving stopped: `None` if it was
/// still running, in which case it has been killed.
fn ping_after_ctrl_c(budget: Duration) -> Option<ExitStatus> {
    let mut attached = condrv::spawn_attached(&command("ping.exe", &["-n", "30", "127.0.0.1"]))
        .expect("spawn_attached");
    let waker = Waker::new().expect("Waker::new");
    let started = Instant::now();
    let mut sent = false;
    let (_, status) = serve_for(&mut attached, &waker, budget, |seen, pid| {
        if sent || seen.connect_pids.is_empty() || started.elapsed() < Duration::from_millis(1500) {
            return;
        }
        sent = true;
        // `send_ctrl` may block on the target's handler, so it runs off
        // the serving thread, as the host's ctrl worker does.
        std::thread::spawn(move || condrv::send_ctrl(pid, CtrlEvent::C).expect("send_ctrl"));
    });
    assert!(sent, "ping exited before Ctrl+C was sent");
    if status.is_none() {
        attached.child.kill_single(Signal::Kill).expect("kill ping");
    }
    status
}

#[test]
fn ctrl_c_reaches_an_attached_child() {
    let _serial = lock();
    accept_ctrl_c();
    let started = Instant::now();
    let status = ping_after_ctrl_c(BUDGET);
    assert!(
        status.is_some(),
        "ping was still running {BUDGET:?} after Ctrl+C"
    );
    assert!(
        started.elapsed() < Duration::from_secs(14),
        "ping ran its course instead of stopping"
    );
}

/// Set in the helper process that [`the_callers_ctrl_c_policy_is_kept`]
/// starts.
const POLICY_HELPER: &str = "PLATFORM_WINDOWS_CONDRV_POLICY_HELPER";

/// A caller that ignores Ctrl+C still ignores it after a failed and a
/// successful `spawn_attached`, so a later child inherits the same
/// policy. Run in a helper started with `CREATE_NEW_PROCESS_GROUP`, so it
/// begins with Ctrl+C ignored whatever the runner does.
#[test]
fn the_callers_ctrl_c_policy_is_kept() {
    if std::env::var_os(POLICY_HELPER).is_some() {
        policy_helper();
        return;
    }
    let _serial = lock();
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let exe = std::env::current_exe().expect("current_exe");
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            "the_callers_ctrl_c_policy_is_kept",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(POLICY_HELPER, "1")
        .creation_flags(CREATE_NEW_PROCESS_GROUP)
        .output()
        .expect("start the helper");
    assert!(
        out.status.success(),
        "helper failed ({:?}):\n{}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn policy_helper() {
    // A spawn that fails at CreateProcessW: the directory doesn't exist.
    let missing = std::env::temp_dir().join("platform-windows-condrv-no-such-dir");
    let mut bad = command("cmd.exe", &["/c", "exit", "0"]);
    bad.cwd = missing.into();
    assert!(
        condrv::spawn_attached(&bad).is_err(),
        "spawn in a missing directory"
    );

    // A spawn that succeeds and runs to completion.
    let mut ok =
        condrv::spawn_attached(&command("cmd.exe", &["/c", "exit", "0"])).expect("spawn_attached");
    let waker = Waker::new().expect("Waker::new");
    let (_, status) = serve(&mut ok, &waker, |_, _| {});
    assert_eq!(status, Some(ExitStatus::Code(0)));

    // A later child inherits the caller's policy, so it ignores Ctrl+C.
    let status = ping_after_ctrl_c(Duration::from_millis(4500));
    assert_eq!(
        status, None,
        "Ctrl+C stopped ping: the caller's policy changed"
    );
}
