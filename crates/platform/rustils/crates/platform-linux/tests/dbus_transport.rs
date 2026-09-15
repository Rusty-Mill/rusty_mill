//! Live D-Bus transport test (rustils#77): spins up a real
//! `dbus-daemon --session` as a test fixture and exercises the hand-
//! rolled client against it directly — no mock, no fake protocol
//! responder. `dbus-daemon` is present in this environment and
//! installable via `apt-get install dbus` in CI (the `dbus`/
//! `dbus-daemon` package alone — a *secret-service* provider like
//! `gnome-keyring-daemon` is a separate, larger question left to
//! rustils#78, not this transport-only slice).

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use platform::error::ErrorKind;
use platform_linux::sys::dbus::{Connection, Value};

/// Kills the spawned `dbus-daemon` on drop, so a test failure (panic)
/// doesn't leak the process.
struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_session_bus() -> (DaemonGuard, String) {
    let mut child = Command::new("dbus-daemon")
        .arg("--session")
        .arg("--nofork")
        .arg("--print-address")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("dbus-daemon must be installed for this test (apt-get install dbus)");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("read dbus-daemon's printed address");
    let address = line.trim().to_string();
    assert!(!address.is_empty(), "dbus-daemon printed no address");
    (DaemonGuard(child), address)
}

#[test]
fn ping_round_trips_against_a_real_session_bus() {
    let (_guard, address) = spawn_session_bus();
    let mut conn = Connection::connect_to(&address).expect("connect + SASL handshake");

    let reply = conn
        .call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.Peer",
            "Ping",
            "",
            vec![],
        )
        .expect("Ping must succeed against a real bus");
    assert!(reply.body.is_empty());
}

#[test]
fn get_id_returns_a_real_string() {
    let (_guard, address) = spawn_session_bus();
    let mut conn = Connection::connect_to(&address).expect("connect + SASL handshake");

    let reply = conn
        .call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetId",
            "",
            vec![],
        )
        .expect("GetId must succeed");
    assert_eq!(reply.body.len(), 1);
    let Value::String(id) = &reply.body[0] else {
        panic!("GetId must return a single STRING, got {:?}", reply.body[0]);
    };
    assert!(!id.is_empty(), "bus id must be non-empty");
}

#[test]
fn calling_an_unknown_method_returns_a_dbus_error() {
    let (_guard, address) = spawn_session_bus();
    let mut conn = Connection::connect_to(&address).expect("connect + SASL handshake");

    let err = conn
        .call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ThisMethodDoesNotExist",
            "",
            vec![],
        )
        .expect_err("an unknown method must come back as a D-Bus ERROR reply, not Ok");
    assert_eq!(err.kind, ErrorKind::Other);
}

#[test]
fn two_independent_connections_both_work_concurrently() {
    // Guards against a bug where transport state is accidentally shared
    // (e.g. a `static`/global serial counter) rather than per-`Connection`.
    let (_guard, address) = spawn_session_bus();
    let mut a = Connection::connect_to(&address).expect("connect a");
    let mut b = Connection::connect_to(&address).expect("connect b");

    for conn in [&mut a, &mut b] {
        conn.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.Peer",
            "Ping",
            "",
            vec![],
        )
        .expect("Ping must succeed on each independent connection");
    }
}
