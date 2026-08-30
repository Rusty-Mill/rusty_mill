//! Live Secret Service test (rustils#78): spins up a real
//! `dbus-daemon --session` + `gnome-keyring-daemon --unlock
//! --components=secrets` pair as test fixtures and exercises the real
//! protocol implementation against them directly — no mock, no fake
//! Secret Service responder. Both binaries are present in this
//! environment and installable via `apt-get install dbus gnome-keyring`
//! in CI.
//!
//! Uses `secret_service::{available_at, get_at, set_at}` (an explicit
//! D-Bus address) rather than the env-discovering `available`/`get`/
//! `set` `LinuxCredentialStore` itself calls — those two paths share
//! every line of protocol logic (`prepare_session_over`/`get_with`/
//! `set_with`), only the connection step differs, so this covers the
//! real behavior fully without mutating the process-wide
//! `DBUS_SESSION_BUS_ADDRESS` environment variable (unsound under
//! parallel test threads, which `cargo test` uses by default).

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use platform::security::CredentialStoreStatus;
use platform_linux::sys::secret_service::{available_at, get_at, set_at};

struct DaemonGuard(Vec<Child>);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_tempdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rustils-secret-service-test-{}-{}",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("mk tempdir");
    dir
}

/// A session bus config identical to the stock `/usr/share/dbus-1/
/// session.conf` except it omits `<standard_session_servicedirs/>` —
/// the directive that makes dbus-daemon auto-activate
/// `/usr/share/dbus-1/services/org.freedesktop.secrets.service`
/// (`gnome-keyring-daemon --start --foreground --components=secrets`,
/// no `--unlock`, no `--replace`, no our custom `XDG_RUNTIME_DIR`/
/// `HOME`) the moment anything calls a method on `org.freedesktop.
/// secrets` before our own explicitly-launched, properly-unlocked
/// daemon has registered that name. That race is real and was caught
/// live: this test's own readiness-polling loop below was, on its very
/// first iteration, enough to trigger activation of the wrong instance,
/// which then permanently held the name (no `--replace` on the
/// activated side, so ours would fail to register afterward) —
/// `another secret service is running`. Disabling activation for this
/// test's own dedicated bus removes the race entirely rather than
/// papering over it with a head-start sleep that would just narrow the
/// window instead of closing it.
fn write_no_activation_session_conf() -> std::path::PathBuf {
    let path = fresh_tempdir().join("session-no-activation.conf");
    std::fs::write(
        &path,
        r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <keep_umask/>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
    )
    .expect("write session config");
    path
}

/// Spawn a real `dbus-daemon` (activation disabled — see
/// `write_no_activation_session_conf`) plus a `gnome-keyring-daemon`
/// unlocked against a fresh, throwaway keyring (a fresh
/// `XDG_RUNTIME_DIR`/`HOME` every call, so no test run ever touches real
/// user data or collides with another test's keyring), and poll until
/// the Secret Service is actually reachable before returning — a fixed
/// sleep would be flaky.
fn spawn_secret_service() -> (DaemonGuard, String) {
    let config = write_no_activation_session_conf();
    let mut dbus = Command::new("dbus-daemon")
        .arg(format!("--config-file={}", config.display()))
        .arg("--nofork")
        .arg("--print-address")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("dbus-daemon must be installed (apt-get install dbus)");
    let stdout = dbus.stdout.take().expect("piped stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("read dbus-daemon's printed address");
    let address = line.trim().to_string();
    assert!(!address.is_empty(), "dbus-daemon printed no address");

    let mut keyring = Command::new("gnome-keyring-daemon")
        .arg("--unlock")
        .arg("--components=secrets")
        .arg("--replace")
        .arg("--foreground")
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env("XDG_RUNTIME_DIR", fresh_tempdir())
        .env("HOME", fresh_tempdir())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("gnome-keyring-daemon must be installed (apt-get install gnome-keyring)");
    // An empty "password" for a fresh throwaway keyring — nothing to
    // remember, since this test's whole point is to never touch a real
    // user keyring. Dropping the returned `ChildStdin` closes the pipe
    // (EOF), the same as the shell `echo "" | gnome-keyring-daemon ...`
    // idiom this mirrors.
    keyring
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(b"\n")
        .expect("write the empty password line");

    let guard = DaemonGuard(vec![dbus, keyring]);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if available_at(&address) == CredentialStoreStatus::Available {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "gnome-keyring-daemon never became reachable within 10s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    (guard, address)
}

#[test]
fn available_reports_unavailable_with_no_bus_reachable() {
    let status = available_at("unix:path=/nonexistent/rustils-test-bus");
    assert_eq!(status, CredentialStoreStatus::Unavailable);
}

#[test]
fn available_reports_available_against_a_real_keyring() {
    let (_guard, address) = spawn_secret_service();
    assert_eq!(available_at(&address), CredentialStoreStatus::Available);
}

#[test]
fn get_is_a_clean_miss_when_nothing_is_stored() {
    let (_guard, address) = spawn_secret_service();
    let result = get_at(&address, "rustils-test-svc", "nobody").expect("get must succeed");
    assert_eq!(result, None);
}

#[test]
fn set_then_get_round_trips_a_real_secret() {
    let (_guard, address) = spawn_secret_service();
    set_at(&address, "rustils-test-svc", "alice", b"hunter2").expect("set must succeed");
    let got = get_at(&address, "rustils-test-svc", "alice").expect("get must succeed");
    assert_eq!(got, Some(b"hunter2".to_vec()));
}

#[test]
fn set_distinguishes_accounts_under_the_same_service() {
    let (_guard, address) = spawn_secret_service();
    set_at(&address, "rustils-test-svc", "alice", b"alice-secret").unwrap();
    set_at(&address, "rustils-test-svc", "bob", b"bob-secret").unwrap();
    assert_eq!(
        get_at(&address, "rustils-test-svc", "alice").unwrap(),
        Some(b"alice-secret".to_vec())
    );
    assert_eq!(
        get_at(&address, "rustils-test-svc", "bob").unwrap(),
        Some(b"bob-secret".to_vec())
    );
}

#[test]
fn set_replaces_an_existing_secret_under_the_same_name() {
    let (_guard, address) = spawn_secret_service();
    set_at(&address, "rustils-test-svc", "alice", b"first").unwrap();
    set_at(&address, "rustils-test-svc", "alice", b"second").unwrap();
    assert_eq!(
        get_at(&address, "rustils-test-svc", "alice").unwrap(),
        Some(b"second".to_vec())
    );
}

#[test]
fn round_trips_binary_data_not_just_text() {
    let (_guard, address) = spawn_secret_service();
    let secret: Vec<u8> = (0..=255u8).collect();
    set_at(&address, "rustils-test-svc", "binary", &secret).unwrap();
    assert_eq!(
        get_at(&address, "rustils-test-svc", "binary").unwrap(),
        Some(secret)
    );
}
