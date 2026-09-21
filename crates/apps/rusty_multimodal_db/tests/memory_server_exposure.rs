//! `ADR-0094` (`EXP-FR-002`/`EXP-FR-003`) against the compiled
//! `memory_server`: a non-loopback bind with nothing configured refuses
//! to start and says what is missing; with a token but no TLS it still
//! refuses, naming TLS; with `SERVER_ALLOW_INSECURE=1` it starts and
//! serves; a loopback bind with nothing configured starts as every
//! version before did (every other subprocess test in this crate is
//! that case).

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn memory_server(bind: &str, env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory_server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SERVER_") {
            command.env_remove(name);
        }
    }
    command
        .arg(bind)
        .env("SERVER_DATA_DIR", unique_dir("memory_server_exposure"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (k, v) in env {
        command.env(k, v);
    }
    command
}

fn wait_for_listener(addr: SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(addr).is_ok() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("memory_server never started listening on {addr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn refusal(bind: &str, env: &[(&str, &str)]) -> String {
    let output = memory_server(bind, env).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        !output.status.success(),
        "memory_server started on {bind} with {env:?}; stderr: {stderr}"
    );
    assert!(
        stderr.contains("refusing to listen on"),
        "no refusal in stderr: {stderr}"
    );
    stderr
}

#[test]
fn an_exposed_bind_with_nothing_configured_refuses_and_names_the_missing_auth() {
    let bind = format!("0.0.0.0:{}", free_port());
    let stderr = refusal(&bind, &[]);
    assert!(
        stderr.contains("SERVER_AUTH_READ_WRITE_TOKEN"),
        "the refusal did not name the token variable: {stderr}"
    );
    assert!(
        stderr.contains("SERVER_ALLOW_INSECURE=1"),
        "the refusal did not name the override: {stderr}"
    );
}

#[test]
fn an_exposed_bind_with_a_token_but_no_tls_refuses_and_names_tls() {
    let bind = format!("0.0.0.0:{}", free_port());
    let stderr = refusal(&bind, &[("SERVER_AUTH_READ_WRITE_TOKEN", "secret")]);
    assert!(
        stderr.contains("SERVER_TLS_CERT_CHAIN_PATH"),
        "the refusal did not name the TLS variable: {stderr}"
    );
}

#[test]
fn allow_insecure_turns_the_refusal_into_a_warning_and_the_server_serves() {
    let port = free_port();
    let bind = format!("0.0.0.0:{port}");
    let child = ChildGuard(
        memory_server(&bind, &[("SERVER_ALLOW_INSECURE", "1")])
            .spawn()
            .unwrap(),
    );
    wait_for_listener(format!("127.0.0.1:{port}").parse().unwrap());
    drop(child);
}
