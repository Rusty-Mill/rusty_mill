//! Reconnecting, against a real server reached through a stand-in `ssh`,
//! and the `--ssh` forward itself. Linux only: `--ssh` is Unix only, and
//! the stand-in is a shell script. The tests against a scripted server,
//! which also run on Windows, are in `scripted.rs`.

#![cfg(target_os = "linux")]

mod common;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use platform::process::EnvSpec;
use platform::pty::PtyMaster;
use rusty_mux_core::{BlockId, WindowId};
use rusty_mux_proto::{Entry, Features, Message};
use rusty_mux_tui::Conn;
use rusty_term_core::{AnsiParser, Grid};

use common::*;

#[test]
fn an_ssh_attach_goes_through_the_forward_and_cleans_up() {
    let work = std::env::temp_dir().join(format!("rusty-mux-fake-ssh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir(&work).expect("work dir");
    let fake = work.join("ssh");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > {w}/args\necho $$ > {w}/pid\n\
         while [ \"$1\" != -L ]; do shift; done\n\
         ln -s \"${{2#*:}}\" \"${{2%%:*}}\"\nexec sleep 600\n",
        w = work.display()
    );
    std::fs::write(&fake, script).expect("fake ssh");
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .expect("chmod");
    let mut env: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    env.insert("RUSTY_MUX_SSH".into(), fake.into_os_string());

    let (socket, server) = scripted_server(true, End::Open);
    let args = [
        OsStr::new("--ssh"),
        OsStr::new("box"),
        socket.as_os_str(),
        BLOCK.as_ref(),
    ];
    let tui = Tui::run(&args, EnvSpec::Explicit(env));
    tui.wait_for(b"REMOTE");
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    server.join().expect("server");
    assert_restored(&screen, "detached from block");

    let args = std::fs::read_to_string(work.join("args")).expect("args");
    let args: Vec<&str> = args.lines().collect();
    assert_eq!(args[args.len() - 2..], ["--", "box"], "{args:?}");
    let forward = args.iter().find(|a| a.contains(':')).expect("-L argument");
    let (local, remote) = forward.split_once(':').expect("local:remote");
    assert_eq!(Path::new(remote), socket);
    let dir = Path::new(local).parent().expect("socket dir");
    assert!(!dir.exists(), "{} was left behind", dir.display());
    let pid = std::fs::read_to_string(work.join("pid")).expect("pid");
    let proc = Path::new("/proc").join(pid.trim());
    assert!(!proc.exists(), "ssh {} still runs", pid.trim());
    std::fs::remove_dir_all(&work).expect("cleanup");
}

// Reconnecting after a lost path, against a real server reached through a
// stand-in `ssh` that forwards like OpenSSH: it relays each connection to
// the local socket on to the server's, so stopping it freezes the path as
// a dead network does, and killing it ends every connection.

/// A real server on a new socket, with `/bin/sh` for each block.
fn real_server() -> PathBuf {
    real_server_with(None).0
}

/// [`real_server`], offering UDP on 127.0.0.1; with its UDP port.
fn real_udp_server() -> (PathBuf, u16) {
    real_udp_server_idle(Duration::from_secs(600))
}

/// A real server offering UDP, whose associations go after `idle` of
/// silence; with the UDP port it took.
fn real_udp_server_idle(idle: Duration) -> (PathBuf, u16) {
    let (path, port) = real_server_with(Some(idle));
    (path, port.expect("udp port"))
}

/// A real server, offering UDP on 127.0.0.1 with associations kept for
/// `udp`'s idle time, when given; with the UDP port it took.
fn real_server_with(udp: Option<Duration>) -> (PathBuf, Option<u16>) {
    use rusty_mux_server::{Config, ConsoleHost, Udp, serve, serve_udp};
    use rusty_tokio::io::{UdpSocket, UnixListener};
    use rusty_tokio::{Builder, LocalOptions};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rusty-mux-tui-real-{}-{n}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let (ready_tx, ready_rx) = mpsc::channel();
    let bind = path.clone();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let listener = UnixListener::bind(&bind).expect("bind");
            let config = Config {
                program: "/bin/sh".into(),
                args: Vec::new(),
                cwd: "/".into(),
                console_host: ConsoleHost::default(),
                state_dir: None,
                grapheme_clustering: false,
                config_file: None,
            };
            let Some(idle) = udp else {
                ready_tx.send(None).expect("ready");
                return serve(listener, config).await.expect("serve");
            };
            let socket = UdpSocket::bind("127.0.0.1:0").await.expect("udp");
            let port = socket.local_addr().expect("udp addr").port();
            ready_tx.send(Some(port)).expect("ready");
            serve_udp(listener, config, Udp { socket, idle })
                .await
                .expect("serve");
        });
    });
    let port = ready_rx.recv_timeout(PATIENCE).expect("server bound");
    (path, port)
}

/// A UDP path to the server's `port` on 127.0.0.1, offered at the same
/// port on 127.0.0.9 (set `STANDIN_HOSTNAME` to that); dropping it, or
/// [`Wire::cut`], silences it both ways, as a dead network does.
struct Wire(Arc<AtomicBool>);

impl Wire {
    fn new(port: u16) -> Self {
        use std::net::UdpSocket;

        let near = UdpSocket::bind(("127.0.0.9", port)).expect("near");
        let far = UdpSocket::bind("127.0.0.1:0").expect("far");
        far.connect(("127.0.0.1", port)).expect("connect");
        let cut = Arc::new(AtomicBool::new(false));
        let client = Arc::new(Mutex::new(None));
        let (out, back) = (
            near.try_clone().expect("near"),
            far.try_clone().expect("far"),
        );
        let (cut_out, client_out) = (Arc::clone(&cut), Arc::clone(&client));
        std::thread::spawn(move || {
            let mut buf = [0; 65536];
            while let Ok((n, from)) = out.recv_from(&mut buf) {
                *client_out.lock().expect("client") = Some(from);
                if !cut_out.load(Ordering::Relaxed) {
                    let _ = back.send(&buf[..n]);
                }
            }
        });
        let cut_in = Arc::clone(&cut);
        std::thread::spawn(move || {
            let mut buf = [0; 65536];
            while let Ok(n) = far.recv(&mut buf) {
                let to = *client.lock().expect("client");
                if let Some(to) = to.filter(|_| !cut_in.load(Ordering::Relaxed)) {
                    let _ = near.send_to(&buf[..n], to);
                }
            }
        });
        Self(cut)
    }

    fn cut(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn mend(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        self.cut();
    }
}

/// A work directory holding a stand-in `ssh` that logs `pid args...` to
/// `log` and relays the `-L` local socket to the remote one. Its `n`th
/// start (from 0) follows line `n` of `plan`, if there is one:
///
/// - `fail`: exits at once, as a refused login does.
/// - `hold`: accepts connections but passes nothing on, either way.
/// - `stall C F`: relays, but passes only the first `F` of the server's
///   messages on its `C`th connection (from 0), and holds the rest.
/// - `trickle C`: relays, but on its `C`th connection sends the client,
///   in place of the server's messages, a message that never ends: its
///   length, then a byte every 20 ms.
/// - `delay MS`: relays, but holds everything it passes on, either way,
///   for `MS` milliseconds, in order: a slow path.
///
/// SIGUSR1 cuts every connection on the client's side only, so the client
/// sees them end while the server keeps its clients. `-G` prints only a
/// `hostname` line, `$STANDIN_HOSTNAME` or 127.0.0.1, and is not a start.
fn relaying_ssh(name: &str, plan: &[&str]) -> PathBuf {
    let work = std::env::temp_dir().join(format!("rusty-mux-relay-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir(&work).expect("work dir");
    std::fs::write(work.join("plan"), plan.join("\n")).expect("plan");
    let script = format!(
        r#"#!/usr/bin/env python3
import os, queue, signal, socket, sys, threading, time
args = sys.argv[1:]
if "-G" in args:
    print("hostname " + os.environ.get("STANDIN_HOSTNAME", "127.0.0.1"))
    sys.exit(0)
with open("{log}", "a") as log:
    log.write(" ".join([str(os.getpid())] + args) + "\n")
with open("{log}") as log:
    start = len(log.read().splitlines()) - 1
with open("{plan}") as plan:
    plans = plan.read().splitlines()
mode = (plans[start] if start < len(plans) else "relay").split()
if mode[0] == "fail":
    sys.exit(255)
stall = (int(mode[1]), int(mode[2])) if mode[0] == "stall" else None
trickle = int(mode[1]) if mode[0] == "trickle" else None
delay = int(mode[1]) / 1000 if mode[0] == "delay" else 0
local, remote = args[args.index("-L") + 1].split(":", 1)
listener = socket.socket(socket.AF_UNIX)
listener.bind(local)
listener.listen()
nears = []
cut = False
def on_cut(*_):
    global cut
    cut = True
    for near in nears:
        try:
            near.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
signal.signal(signal.SIGUSR1, on_cut)
def drip(near):
    try:
        near.sendall((60000).to_bytes(4, "big"))
        while True:
            near.sendall(b"\0")
            time.sleep(0.02)
    except OSError:
        pass
def late(b, q):
    try:
        while True:
            due, data = q.get()
            time.sleep(max(0, due - time.monotonic()))
            if data is None:
                break
            b.sendall(data)
    except OSError:
        pass
def pump(a, b, frames=None):
    held = b""
    q = queue.Queue()
    sender = threading.Thread(target=late, args=(b, q), daemon=True)
    if delay:
        sender.start()
    try:
        while True:
            data = a.recv(65536)
            if not data:
                break
            if delay:
                q.put((time.monotonic() + delay, data))
                continue
            if frames is None:
                b.sendall(data)
                continue
            held += data
            while frames > 0 and len(held) >= 4:
                end = 4 + int.from_bytes(held[:4], "big")
                if len(held) < end:
                    break
                b.sendall(held[:end])
                held = held[end:]
                frames -= 1
    except OSError:
        pass
    if delay:
        q.put((time.monotonic() + delay, None))
        sender.join()
    for end in (a, b):
        if cut and end not in nears:
            continue
        try:
            end.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
held = []
while True:
    near, _ = listener.accept()
    nears.append(near)
    if mode[0] == "hold":
        held.append(near)
        continue
    far = socket.socket(socket.AF_UNIX)
    far.connect(remote)
    if trickle == len(nears) - 1:
        threading.Thread(target=drip, args=(near,), daemon=True).start()
        threading.Thread(target=pump, args=(near, far), daemon=True).start()
        continue
    frames = stall[1] if stall and stall[0] == len(nears) - 1 else None
    threading.Thread(target=pump, args=(near, far), daemon=True).start()
    threading.Thread(target=pump, args=(far, near, frames), daemon=True).start()
"#,
        log = work.join("log").display(),
        plan = work.join("plan").display(),
    );
    let ssh = work.join("ssh");
    std::fs::write(&ssh, script).expect("stand-in ssh");
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .expect("chmod");
    work
}

/// Each start of the stand-in so far: its pid and arguments.
fn logins(work: &Path) -> Vec<(u32, String)> {
    let log = std::fs::read_to_string(work.join("log")).unwrap_or_default();
    log.lines()
        .filter_map(|line| {
            let (pid, args) = line.split_once(' ')?;
            Some((pid.parse().ok()?, args.to_owned()))
        })
        .collect()
}

/// Waits, up to `patience`, for the stand-in's `n`th start.
fn wait_logins(work: &Path, n: usize, patience: Duration) -> Vec<(u32, String)> {
    let deadline = Instant::now() + patience;
    loop {
        let seen = logins(work);
        if seen.len() >= n {
            return seen;
        }
        assert!(
            Instant::now() < deadline,
            "only {} logins: {seen:?}",
            seen.len()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn signal(pid: u32, signal: &str) {
    let status = std::process::Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .expect("kill");
    assert!(status.success(), "kill {signal} {pid}");
}

fn running(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

/// The client through the stand-in in `work` to the server at `socket`,
/// on a new block.
fn ssh_client(work: &Path, socket: &Path) -> Tui {
    ssh_client_with(work, socket, &[])
}

/// [`ssh_client`] with `extra` set in its environment.
fn ssh_client_with(work: &Path, socket: &Path, extra: &[(&str, &str)]) -> Tui {
    let mut env: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
    env.insert("RUSTY_MUX_SSH".into(), work.join("ssh").into_os_string());
    for (key, value) in extra {
        env.insert(key.into(), value.into());
    }
    let args = [OsStr::new("--ssh"), OsStr::new("box"), socket.as_os_str()];
    Tui::run(&args, EnvSpec::Explicit(env))
}

/// Types `command` until `needle` shows: keys typed before the client
/// holds the block's lease are not the block's.
fn type_until(tui: &Tui, command: &[u8], needle: &[u8]) {
    let deadline = Instant::now() + PATIENCE;
    while !tui.saw(needle) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {needle:?}"
        );
        tui.master.write(command).expect("type");
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// A path that goes silent, as on a network change, is noticed by the
/// heartbeat and brought back with a second login that may not prompt;
/// the client takes its clients over, so the block's screen comes back
/// and typing reaches it without asking for the lease again. The frozen
/// first `ssh` is stopped only after the takeover, and nothing is left
/// once the client detaches.
#[test]
fn a_frozen_path_is_brought_back_with_the_screen_and_the_lease() {
    let socket = real_server();
    let work = relaying_ssh("frozen", &[]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    let first = wait_logins(&work, 1, PATIENCE)[0].0;
    signal(first, "-STOP");
    let frozen = tui.written();

    // Noticed after 15 s without a sign of life, then logged in again.
    let seen = wait_logins(&work, 2, Duration::from_secs(30));
    assert!(!seen[0].1.contains("BatchMode"), "{seen:?}");
    assert!(seen[1].1.contains("BatchMode=yes"), "{seen:?}");
    // The takeover sends the block's screen again; keys typed before it
    // are dropped, since the path is still down.
    let deadline = Instant::now() + PATIENCE;
    while !tui.since(frozen).windows(5).any(|w| w == b"one-1") {
        assert!(Instant::now() < deadline, "the screen did not come back");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Typed once: the lease came along, so the first keys count.
    let from = tui.written();
    tui.master.write(b"echo two-$((1+1))\r").expect("type");
    tui.wait_for(b"two-2");
    let echoed = tui.since(from);
    let twice = echoed.windows(5).filter(|w| *w == b"two-2").count();
    assert_eq!(twice, 1, "{:?}", String::from_utf8_lossy(&echoed));
    assert!(!running(first), "the frozen ssh outlived the takeover");

    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit();
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("detached from block"), "{text:?}");
    assert!(!text.contains("path to the server was down"), "{text:?}");
    assert!(!running(seen[1].0), "the second ssh outlived the client");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// A server that offers UDP gets the client's connections over it: the
/// client moves once the association answers, and the forward stops. A
/// block shown later opens its own stream, and typing reaches the block.
#[test]
fn an_ssh_attach_moves_to_udp_and_stops_the_forward() {
    let socket = real_udp_server().0;
    let work = relaying_ssh("udp", &[]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    let forward = wait_logins(&work, 1, PATIENCE)[0].0;
    let deadline = Instant::now() + PATIENCE;
    while running(forward) {
        assert!(Instant::now() < deadline, "the forward outlived the move");
        std::thread::sleep(Duration::from_millis(50));
    }
    // A split shows a new block, on a new stream.
    tui.master.write(b"\x1d|").expect("ctrl-] |");
    type_until(&tui, b"echo two-$((1+1))\r", b"two-2");

    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let text = String::from_utf8_lossy(&tui.exit()).into_owned();
    assert!(text.contains("detached from block"), "{text:?}");
    assert!(!text.contains("staying on SSH"), "{text:?}");
    assert_eq!(logins(&work).len(), 1, "no second login");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// A client moved onto UDP through `wire`, whose forward stopped once
/// the move was done.
fn on_udp(work: &Path, socket: &Path) -> Tui {
    let tui = ssh_client_with(work, socket, &[("STANDIN_HOSTNAME", "127.0.0.9")]);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    stopped(wait_logins(work, 1, PATIENCE)[0].0);
    tui
}

/// Waits until the stand-in `pid` has stopped.
fn stopped(pid: u32) {
    let deadline = Instant::now() + PATIENCE;
    while running(pid) {
        assert!(Instant::now() < deadline, "ssh {pid} kept running");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A UDP path that goes silent is lost after 6 s without a datagram,
/// two of the server's keepalives, not after the 15 s a forward needs:
/// the probe of the dead association fails, a second login brings the
/// clients back over SSH, and typing reaches the block.
#[test]
fn a_silent_udp_path_is_noticed_in_6_s() {
    let (socket, port) = real_udp_server();
    let wire = Wire::new(port);
    let work = relaying_ssh("udp-silent", &[]);
    let tui = on_udp(&work, &socket);
    wire.cut();
    let cut = Instant::now();

    // 6 s of silence and the 2 s probe; a missed sign of life takes 10 s
    // and more, and the probe on top of it.
    wait_logins(&work, 2, Duration::from_secs(11));
    assert!(
        cut.elapsed() < Duration::from_secs(11),
        "{:?}",
        cut.elapsed()
    );
    type_until(&tui, b"echo two-$((1+1))\r", b"two-2");
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// An idle UDP path that still works is not lost: with nothing typed or
/// shown for longer than the 6 s of silence and the probe, the server's
/// keepalives keep it, so there is no second login and typing still
/// reaches the block over UDP.
#[test]
fn an_idle_udp_path_is_not_lost() {
    let (socket, port) = real_udp_server();
    let _wire = Wire::new(port);
    let work = relaying_ssh("udp-idle", &[]);
    let tui = on_udp(&work, &socket);
    std::thread::sleep(Duration::from_secs(9));
    assert_eq!(logins(&work).len(), 1, "an idle path was given up");
    type_until(&tui, b"echo two-$((1+1))\r", b"two-2");
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    assert!(!text.contains("path to the server was down"), "{text:?}");
    assert_eq!(logins(&work).len(), 1, "no second login");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// After a fallback to SSH, the association given up is kept: once it is
/// heard again the clients move back onto it, the second forward stops,
/// and typing reaches the block with no third login.
#[test]
fn a_udp_path_heard_again_after_a_fallback_gets_the_clients_back() {
    let (socket, port) = real_udp_server();
    let wire = Wire::new(port);
    let work = relaying_ssh("udp-back", &[]);
    let tui = on_udp(&work, &socket);
    wire.cut();
    let second = wait_logins(&work, 2, Duration::from_secs(20))[1].0;
    type_until(&tui, b"echo two-$((1+1))\r", b"two-2");
    assert!(running(second), "the fallback forward stopped early");

    // A keepalive within 3 s, the probe, then the takeover.
    wire.mend();
    stopped(second);
    type_until(&tui, b"echo three-$((1+2))\r", b"three-3");
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    assert!(!text.contains("path to the server was down"), "{text:?}");
    assert_eq!(logins(&work).len(), 2, "no third login");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// A path in standby the server let go, past its idle time, is never
/// heard again: the client asks for a new association instead, and moves
/// onto it once UDP answers, with no third login.
#[test]
fn a_standby_the_server_let_go_is_replaced_by_a_new_association() {
    // Past the fallback (6 s silence, a 2 s probe), but soon after.
    let idle = Duration::from_secs(12);
    let (socket, port) = real_udp_server_idle(idle);
    let wire = Wire::new(port);
    let work = relaying_ssh("udp-expired", &[]);
    let tui = on_udp(&work, &socket);
    wire.cut();
    let cut = Instant::now();
    let second = wait_logins(&work, 2, Duration::from_secs(20))[1].0;
    type_until(&tui, b"echo two-$((1+1))\r", b"two-2");

    std::thread::sleep((idle + Duration::from_secs(2)).saturating_sub(cut.elapsed()));
    wire.mend();
    // Asked again at most 30 s apart.
    let deadline = Instant::now() + Duration::from_secs(40);
    while running(second) {
        assert!(Instant::now() < deadline, "still on SSH");
        std::thread::sleep(Duration::from_millis(50));
    }
    type_until(&tui, b"echo three-$((1+2))\r", b"three-3");
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    assert_eq!(logins(&work).len(), 2, "no third login");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// A client that stayed on SSH because UDP did not answer asks the
/// server for a new association later, and moves onto it once one
/// answers: the forward stops, with no second login.
#[test]
fn a_client_that_stayed_on_ssh_moves_to_udp_once_it_answers() {
    let (socket, port) = real_udp_server();
    let wire = Wire::new(port);
    wire.cut();
    let work = relaying_ssh("udp-later", &[]);
    let tui = ssh_client_with(&work, &socket, &[("STANDIN_HOSTNAME", "127.0.0.9")]);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    let forward = wait_logins(&work, 1, PATIENCE)[0].0;
    std::thread::sleep(Duration::from_secs(3));
    assert!(running(forward), "moved while UDP was cut");

    // Asked again at most 30 s apart; here within the first few tries.
    wire.mend();
    let deadline = Instant::now() + Duration::from_secs(25);
    while running(forward) {
        assert!(Instant::now() < deadline, "still on SSH");
        std::thread::sleep(Duration::from_millis(50));
    }
    type_until(&tui, b"echo two-$((1+1))\r", b"two-2");
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    assert_eq!(logins(&work).len(), 1, "no second login");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// With UDP unanswered, or turned off, the client stays on the forward:
/// in 2 s or not at all, and it says so once when it tried.
#[test]
fn an_unanswered_or_unwanted_udp_keeps_the_client_on_ssh() {
    let socket = real_udp_server().0;
    for (name, env) in [
        // TEST-NET-1: nothing there answers.
        ("udp-blocked", ("STANDIN_HOSTNAME", "192.0.2.1")),
        ("udp-off", ("RUSTY_MUX_UDP", "off")),
    ] {
        let work = relaying_ssh(name, &[]);
        let started = Instant::now();
        let tui = ssh_client_with(&work, &socket, &[env]);
        type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
        assert!(started.elapsed() < PATIENCE, "{name}: too slow");
        let forward = wait_logins(&work, 1, PATIENCE)[0].0;
        assert!(running(forward), "{name}: the forward stopped");

        tui.master.write(b"\x1dd").expect("ctrl-] d");
        let text = String::from_utf8_lossy(&tui.exit()).into_owned();
        assert!(text.contains("detached from block"), "{name}: {text:?}");
        let said = text.matches("staying on SSH").count();
        assert_eq!(said, usize::from(name == "udp-blocked"), "{name}: {text:?}");
        assert!(!running(forward), "{name}: the forward outlived the client");
        std::fs::remove_dir_all(&work).expect("cleanup");
    }
}

/// Reconnecting only ever continues the clients the server still has: one
/// the server let go meanwhile (here, ended by another client) is not
/// replaced by a new one, and the client stops and says so.
#[test]
fn a_client_the_server_let_go_is_not_brought_back() {
    let socket = real_server();
    let work = relaying_ssh("dropped", &[]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    disconnect_everyone(&socket);

    let screen = tui.exit();
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("the server let this client go"), "{text:?}");
    let seen = logins(&work);
    assert_eq!(seen.len(), 2, "one login to try again: {seen:?}");
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// Ends every other client of the server at `socket`, as Disconnect does.
fn disconnect_everyone(socket: &Path) {
    let mut conn = Conn::new(Stream::connect(socket).expect("connect"));
    conn.send(&Message::Hello {
        version: None,
        features: Features::CHECKPOINT_V6
            .union(Features::INSPECT)
            .union(Features::LABELS),
        resume: None,
        takeover: None,
    })
    .expect("hello");
    let me = match conn.recv().expect("welcome") {
        Message::Welcome { client, .. } => client,
        other => panic!("expected Welcome, got {other:?}"),
    };
    conn.send(&Message::List { req: 1 }).expect("list");
    let mut others = Vec::new();
    loop {
        match conn.recv().expect("list answer") {
            Message::Entry {
                entry: Entry::Client { client, .. },
                ..
            } if client != me => others.push(client),
            Message::Done { req: 1, .. } => break,
            _ => {}
        }
    }
    assert!(!others.is_empty(), "no clients to end");
    for (req, client) in (2..).zip(others) {
        conn.send(&Message::Disconnect { req, client })
            .expect("disconnect");
        loop {
            match conn.recv().expect("disconnect answer") {
                Message::Done { req: r, .. } if r == req => break,
                Message::Failed { req: r, reason, .. } if r == req => panic!("{reason}"),
                _ => {}
            }
        }
    }
}

/// What the client's output leaves on an 80x24 terminal, row by row.
fn screen_text(output: &[u8]) -> String {
    let mut grid = Grid::new(80, 24);
    AnsiParser::new().advance(&mut grid, output);
    (0..grid.rows)
        .map(|y| grid.row_cells(y).map(|cell| cell.ch).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Loses the path at once: the first stand-in cuts the client's side of
/// every connection, and the server keeps the clients.
fn cut(work: &Path) {
    signal(wait_logins(work, 1, PATIENCE)[0].0, "-USR1");
}

/// A fresh path that comes up but never answers is given up on within
/// the handshake's bound, its `ssh` stopped, and tried again; the next
/// one brings the block back with its lease.
#[test]
fn a_fresh_path_that_never_answers_is_given_up_and_tried_again() {
    let socket = real_server();
    let work = relaying_ssh("hold", &["relay", "hold"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    cut(&work);
    let held = wait_logins(&work, 2, PATIENCE)[1].0;
    let from = tui.written();
    // 10 s for the answers, then 2 s before trying again.
    let seen = wait_logins(&work, 3, Duration::from_secs(20));
    assert!(!running(held), "the silent ssh was kept");
    let deadline = Instant::now() + PATIENCE;
    while !tui.since(from).windows(5).any(|w| w == b"one-1") {
        assert!(Instant::now() < deadline, "the screen did not come back");
        std::thread::sleep(Duration::from_millis(20));
    }
    let from = tui.written();
    tui.master.write(b"echo two-$((1+1))\r").expect("type");
    tui.wait_for(b"two-2");
    assert!(tui.since(from).windows(5).any(|w| w == b"two-2"));
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// Once the control connection's client is taken over, a fresh path that
/// stops answering (here, before the layout) cannot be given up on and
/// tried again, since hanging it up ends that client: the client stops
/// within the handshake's bound and says why, and both `ssh`s go.
#[test]
fn a_takeover_that_stalls_midway_stops_the_client_in_time() {
    let socket = real_server();
    let work = relaying_ssh("stall-list", &["relay", "stall 0 1"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    cut(&work);
    wait_logins(&work, 2, PATIENCE);
    let screen = tui.exit_within(Duration::from_secs(20));
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("reconnecting: "), "{text:?}");
    let seen = logins(&work);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// Ctrl-] d detaches at once while a block's takeover waits on an answer
/// that never comes, well before the handshake's bound, and both `ssh`s
/// go with the client.
#[test]
fn detaching_while_a_takeover_stalls_is_prompt() {
    let socket = real_server();
    let work = relaying_ssh("stall-pane", &["relay", "stall 1 0"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    cut(&work);
    wait_logins(&work, 2, PATIENCE);
    // Past the login and the control connection's takeover.
    std::thread::sleep(Duration::from_secs(1));
    let asked = Instant::now();
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit_within(Duration::from_secs(3));
    assert!(asked.elapsed() < Duration::from_secs(3));
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("detached from block"), "{text:?}");
    let seen = logins(&work);
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// A split window stays on screen while the path is down and a login
/// fails, and both blocks are drawn again once it is back.
#[test]
fn a_split_window_stays_on_screen_while_the_path_is_down() {
    let socket = real_server();
    let work = relaying_ssh("split", &["relay", "fail"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    tui.master.write(b"\x1d|").expect("ctrl-] |");
    type_until(&tui, b"echo two-$((1+1))\r", b"two-2");
    let shown = |tui: &Tui| {
        let text = screen_text(&tui.since(0));
        text.contains("one-1") && text.contains("two-2")
    };
    let deadline = Instant::now() + PATIENCE;
    while !shown(&tui) {
        assert!(Instant::now() < deadline, "the split did not show both");
        std::thread::sleep(Duration::from_millis(20));
    }
    cut(&work);
    // The failed login, then 2 s before the next.
    wait_logins(&work, 2, PATIENCE);
    std::thread::sleep(Duration::from_millis(500));
    let text = screen_text(&tui.since(0));
    assert!(text.contains("one-1") && text.contains("two-2"), "{text}");

    // Start observing before the successful login can finish its takeover. The
    // login is recorded before the client receives and paints the restored
    // panes, and on a loaded runner that whole repaint can otherwise land in
    // the gap between `wait_logins` returning and this cursor being sampled.
    let from = tui.written();
    wait_logins(&work, 3, PATIENCE);
    let deadline = Instant::now() + PATIENCE;
    loop {
        let again = String::from_utf8_lossy(&tui.since(from)).into_owned();
        if again.contains("one-1") && again.contains("two-2") {
            break;
        }
        assert!(Instant::now() < deadline, "not drawn again: {again:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(shown(&tui), "{}", screen_text(&tui.since(0)));
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    let seen = logins(&work);
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// A fresh path that sends a message slower than it could ever end, a
/// byte at a time, is still given up on within the handshake's bound and
/// tried again.
#[test]
fn a_trickling_answer_is_given_up_in_time() {
    let socket = real_server();
    let work = relaying_ssh("trickle", &["relay", "trickle 0"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    cut(&work);
    let trickling = wait_logins(&work, 2, PATIENCE)[1].0;
    let asked = Instant::now();
    // 10 s for the answers, then 2 s before trying again.
    let seen = wait_logins(&work, 3, Duration::from_secs(20));
    assert!(
        asked.elapsed() < Duration::from_secs(15),
        "{:?}",
        asked.elapsed()
    );
    assert!(!running(trickling), "the trickling ssh was kept");
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// Ctrl-] d detaches at once while an answer trickles in, and both
/// `ssh`s go with the client.
#[test]
fn detaching_while_an_answer_trickles_is_prompt() {
    let socket = real_server();
    let work = relaying_ssh("trickle-detach", &["relay", "trickle 0"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    cut(&work);
    wait_logins(&work, 2, PATIENCE);
    std::thread::sleep(Duration::from_secs(1));
    let asked = Instant::now();
    tui.master.write(b"\x1dd").expect("ctrl-] d");
    let screen = tui.exit_within(Duration::from_secs(3));
    assert!(asked.elapsed() < Duration::from_secs(3));
    let text = String::from_utf8_lossy(&screen);
    assert!(text.contains("detached from block"), "{text:?}");
    let seen = logins(&work);
    assert!(seen.iter().all(|(pid, _)| !running(*pid)), "{seen:?}");
    std::fs::remove_dir_all(&work).expect("cleanup");
}

/// A block hidden by another client's zoom is left, not kept for a
/// reconnect: once shown again, its new client gets the lease at once
/// instead of waiting out the old one's grace.
#[test]
fn a_block_zoomed_away_and_back_is_leased_again_at_once() {
    let socket = real_server();
    let tui = Tui::run(&[socket.as_os_str()], EnvSpec::Inherit);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    tui.master.write(b"\x1d|").expect("ctrl-] |");
    let mut other = control(&socket);
    let (window, left, right) = split_of(&mut other);
    let blocks = [left, right];
    leased(&mut other, blocks, (false, true));
    tui.master.write(b"\x1dh").expect("ctrl-] h");
    leased(&mut other, blocks, (true, false));

    // Zooming the right block hides the focused, leased left one.
    let block = right;
    ask(&mut other, 1, Message::Zoom { req: 1, block });
    leased(&mut other, blocks, (false, true));
    ask(&mut other, 2, Message::Unzoom { req: 2, window });
    // Focusing left does nothing until the client has the unzoom.
    let deadline = Instant::now() + PATIENCE;
    while holders(&mut other, blocks) != (true, false) {
        assert!(Instant::now() < deadline, "the left block was not leased");
        tui.master.write(b"\x1dh").expect("ctrl-] h");
        std::thread::sleep(Duration::from_millis(200));
    }
    type_until(&tui, b"echo two-$((1+1))\r", b"two-2");
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
}

/// A client of the server at `socket` that can change and edit layouts
/// and move the focus.
fn control(socket: &Path) -> Conn<Stream> {
    let mut conn = Conn::new(Stream::connect(socket).expect("connect"));
    conn.send(&Message::Hello {
        version: None,
        features: Features::CHECKPOINT_V6
            .union(Features::INSPECT)
            .union(Features::LAYOUT)
            .union(Features::LAYOUT_EDIT)
            .union(Features::FOCUS),
        resume: None,
        takeover: None,
    })
    .expect("hello");
    match conn.recv().expect("welcome") {
        Message::Welcome { .. } => conn,
        other => panic!("expected Welcome, got {other:?}"),
    }
}

/// Sends `request`, numbered `req`, and waits for its Done.
fn ask(conn: &mut Conn<Stream>, req: u64, request: Message) {
    conn.send(&request).expect("request");
    loop {
        match conn.recv().expect("answer") {
            Message::Done { req: r, .. } if r == req => return,
            Message::Failed { req: r, reason, .. } if r == req => panic!("{reason}"),
            _ => {}
        }
    }
}

/// The server's entries.
fn list(conn: &mut Conn<Stream>) -> Vec<Entry> {
    conn.send(&Message::List { req: 0 }).expect("list");
    let mut entries = Vec::new();
    loop {
        match conn.recv().expect("list answer") {
            Message::Entry { entry, .. } => entries.push(entry),
            Message::Done { req: 0, .. } => return entries,
            _ => {}
        }
    }
}

/// The only window and its two blocks, left then right, once it is split.
fn split_of(conn: &mut Conn<Stream>) -> (WindowId, BlockId, BlockId) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let split = list(conn).into_iter().find_map(|entry| match entry {
            Entry::Window {
                window,
                layout: Some(layout),
                ..
            } => match layout.blocks().as_slice() {
                [left, right] => Some((window, *left, *right)),
                _ => None,
            },
            _ => None,
        });
        if let Some(split) = split {
            return split;
        }
        assert!(Instant::now() < deadline, "not split");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether each of `blocks` is leased.
fn holders(conn: &mut Conn<Stream>, [left, right]: [BlockId; 2]) -> (bool, bool) {
    let entries = list(conn);
    let held = |b: BlockId| {
        entries
            .iter()
            .any(|e| matches!(e, Entry::Block { block, holder: Some(_), .. } if *block == b))
    };
    (held(left), held(right))
}

/// Waits until whether each of `blocks` is leased is `want`.
fn leased(conn: &mut Conn<Stream>, blocks: [BlockId; 2], want: (bool, bool)) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let now = holders(conn, blocks);
        if now == want {
            return;
        }
        assert!(Instant::now() < deadline, "leased {now:?}, not {want:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// How a predicted character is drawn ahead over a plain shell prompt:
/// underlined, in the reset rendition.
const AHEAD: &[u8] = b"\x1b[0;4m";

/// Types `keys` one at a time, `gap` apart.
fn type_slowly(tui: &Tui, keys: &[u8], gap: Duration) {
    for key in keys {
        tui.master.write(std::slice::from_ref(key)).expect("type");
        std::thread::sleep(gap);
    }
}

/// Detaches, then removes the stand-in's work directory.
fn detach_ssh(tui: Tui, work: &Path) {
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
    std::fs::remove_dir_all(work).expect("cleanup");
}

/// Over a path that holds every message 100 ms each way, typing at a
/// shell prompt is drawn underlined ahead of its echo once the line's first
/// key is found on the screen; at a prompt with echo off (`sh` has no
/// `read -s`), nothing typed is ever written to the terminal.
#[test]
fn a_slow_path_draws_typing_ahead_but_never_a_secret() {
    let socket = real_server();
    let work = relaying_ssh("slow", &["delay 100"]);
    let tui = ssh_client(&work, &socket);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    let gap = Duration::from_millis(600);
    // `e` starts the line; once its echo and ack are in, `c` is drawn.
    type_slowly(&tui, b"ec", gap);
    tui.wait_for(&[AHEAD, b"c"].concat());
    type_slowly(&tui, b"ho ok\r", gap);
    tui.wait_for(b"ok\r\n");

    tui.master.write(b"stty -echo; read s\r").expect("type");
    std::thread::sleep(gap * 2);
    let from = tui.written();
    type_slowly(&tui, b"QJXV", gap);
    std::thread::sleep(gap * 2);
    let shown = tui.since(from);
    assert!(
        !shown.iter().any(|b| b"QJXV".contains(b)),
        "{:?}",
        String::from_utf8_lossy(&shown)
    );
    tui.master.write(b"\rstty echo\r").expect("end the read");
    detach_ssh(tui, &work);
}

/// Over a fast path the latency gate keeps predictions off, and over a
/// slow one `RUSTY_MUX_PREDICT=off` does.
#[test]
fn predictions_stay_off_on_a_fast_path_or_when_switched_off() {
    let socket = real_server();
    for (name, plan, extra) in [
        ("fast", "relay", &[][..]),
        ("off", "delay 100", &[("RUSTY_MUX_PREDICT", "off")][..]),
    ] {
        let work = relaying_ssh(name, &[plan]);
        let tui = ssh_client_with(&work, &socket, extra);
        type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
        type_slowly(&tui, b"echo ok\r", Duration::from_millis(600));
        tui.wait_for(b"ok\r\n");
        assert!(!tui.saw(AHEAD), "{name}: drew a prediction");
        detach_ssh(tui, &work);
    }
}

/// The window's layout blocks and focus, and the blocks holding a lease.
#[derive(Debug, Default, PartialEq)]
struct Seen {
    window: Option<WindowId>,
    layout: Vec<BlockId>,
    focused: Option<BlockId>,
    leased: Vec<BlockId>,
    blocks: Vec<BlockId>,
}

fn look(conn: &mut Conn<Stream>) -> Seen {
    let mut seen = Seen::default();
    for entry in list(conn) {
        match entry {
            Entry::Window {
                window,
                layout,
                focused,
                ..
            } => {
                seen.window = Some(window);
                seen.layout = layout.map(|l| l.blocks()).unwrap_or_default();
                seen.focused = focused;
            }
            Entry::Block { block, holder, .. } => {
                seen.blocks.push(block);
                if holder.is_some() {
                    seen.leased.push(block);
                }
            }
            _ => {}
        }
    }
    seen
}

/// Lists until the server shows what `wanted` accepts.
fn until(conn: &mut Conn<Stream>, wanted: impl Fn(&Seen) -> bool) -> Seen {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let seen = look(conn);
        if wanted(&seen) {
            return seen;
        }
        assert!(Instant::now() < deadline, "never: {seen:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The attach client follows the window's shared focus (taking the
/// focused block's lease), edits the layout with its keys, and keeps an
/// empty window open until a block is moved back.
#[test]
fn the_client_follows_the_shared_focus_and_edits_the_layout() {
    let socket = real_server();
    let tui = Tui::run(&[socket.as_os_str()], EnvSpec::Inherit);
    type_until(&tui, b"echo one-$((1+0))\r", b"one-1");
    let mut conn = control(&socket);
    let seen = until(&mut conn, |s| s.leased.len() == 1);
    let a = seen.leased[0];
    let req = 1;
    let split = Message::Split {
        req,
        block: a,
        side: rusty_mux_core::Side::Right,
        launch: rusty_mux_proto::Launch::default(),
    };
    conn.send(&split).expect("split");
    let b = loop {
        match conn.recv().expect("split answer") {
            Message::Done {
                req: r,
                created: Some(id),
            } if r == req => break BlockId::from_uuid(id),
            Message::Failed { reason, .. } => panic!("{reason}"),
            _ => {}
        }
    };

    // An agent's split does not take the focus; focusing B does, and the
    // client follows, leasing B and giving A back.
    let seen = until(&mut conn, |s| s.layout == [a, b]);
    assert_eq!(seen.focused, Some(a));
    ask(&mut conn, 2, Message::FocusBlock { req: 2, block: b });
    until(&mut conn, |s| s.leased == [b]);
    // The client's own focus keys move the shared focus.
    tui.master.write(b"\x1dh").expect("ctrl-] h");
    until(&mut conn, |s| s.focused == Some(a) && s.leased == [a]);
    tui.master.write(b"\x1dl").expect("ctrl-] l");
    until(&mut conn, |s| s.focused == Some(b) && s.leased == [b]);

    // Ctrl-] D detaches the focused block; the focus passes to A.
    tui.master.write(b"\x1dD").expect("ctrl-] D");
    let seen = until(&mut conn, |s| s.layout == [a]);
    assert_eq!(seen.focused, Some(a));
    assert!(seen.blocks.contains(&b), "B keeps running: {seen:?}");
    until(&mut conn, |s| s.leased == [a]);

    // Detaching A too leaves the window empty, and the client open.
    tui.master.write(b"\x1dD").expect("ctrl-] D");
    until(&mut conn, |s| s.layout.is_empty() && s.leased.is_empty());
    std::thread::sleep(Duration::from_secs(1));
    assert!(!tui.saw(b"closed"), "the client must stay open");

    // Moved back by an agent, A shows and is leased again.
    let back = Message::MoveBlock {
        req: 3,
        block: a,
        beside: None,
        side: rusty_mux_core::Side::Right,
    };
    ask(&mut conn, 3, back);
    until(&mut conn, |s| s.leased == [a]);
    let text = detach(tui);
    assert!(text.contains("detached from block"), "{text:?}");
}
