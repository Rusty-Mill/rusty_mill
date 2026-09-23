//! `ADR-0094` (`EXP-FR-002`/`EXP-FR-003`) against the compiled
//! `memory_server`: a non-loopback bind with nothing configured refuses
//! to start and says what is missing; with a token but no TLS it still
//! refuses, naming TLS; with `SERVER_ALLOW_INSECURE=1` it starts and
//! serves; a loopback bind with nothing configured starts as every
//! version before did (every other subprocess test in this crate is
//! that case).

use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

/// `RGT-FR-004` (ADR-0123): a server spawned on port 0, its bound address
/// read from the listening banner — no port is picked ahead of the bind,
/// so two tests can never race for one. `before_banner` is every stderr
/// line the server wrote before it; the pipe stays open for its lifetime.
struct Server {
    child: Child,
    addr: SocketAddr,
    // The same helper in every binary test; each reads the fields it needs.
    #[allow(dead_code)]
    before_banner: String,
    /// The listening banner itself, the server's own account of its settings.
    #[allow(dead_code)]
    banner: String,
    _stderr: std::io::Lines<std::io::BufReader<std::process::ChildStderr>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_listening(mut command: Command) -> Server {
    use std::io::BufRead;
    let mut child = command.stderr(Stdio::piped()).spawn().unwrap();
    let stderr = child.stderr.take().expect("stderr is piped");
    let mut lines = std::io::BufReader::new(stderr).lines();
    let mut before_banner = String::new();
    let (addr, banner) = loop {
        let line = lines
            .next()
            .unwrap_or_else(|| panic!("stderr closed before the listening banner: {before_banner}"))
            .unwrap();
        if let Some(rest) = line.strip_prefix("memory_server listening on ") {
            let addr = rest
                .split(' ')
                .next()
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap();
            break (addr, line);
        }
        before_banner.push_str(&line);
        before_banner.push('\n');
    };
    Server {
        child,
        addr,
        before_banner,
        banner,
        _stderr: lines,
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
    let stderr = refusal("0.0.0.0:0", &[]);
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
    let bind = "0.0.0.0:0".to_string();
    let stderr = refusal(&bind, &[("SERVER_AUTH_READ_WRITE_TOKEN", "secret")]);
    assert!(
        stderr.contains("SERVER_TLS_CERT_CHAIN_PATH"),
        "the refusal did not name the TLS variable: {stderr}"
    );
}

/// `RVM-FR-004` (ADR-0111): the metrics HTTP listener is held to the
/// same rule — a non-loopback metrics bind is refused even when the wire
/// listener is loopback, naming the variable; `SERVER_ALLOW_INSECURE=1`
/// turns it into a warning.
#[test]
fn an_exposed_metrics_bind_refuses_even_when_the_wire_bind_is_loopback() {
    let stderr = refusal("127.0.0.1:0", &[("SERVER_METRICS_HTTP_ADDR", "0.0.0.0:0")]);
    assert!(
        stderr.contains("SERVER_METRICS_HTTP_ADDR"),
        "the refusal did not name the metrics listener: {stderr}"
    );
    let server = spawn_listening(memory_server(
        "127.0.0.1:0",
        &[
            ("SERVER_METRICS_HTTP_ADDR", "0.0.0.0:0"),
            ("SERVER_ALLOW_INSECURE", "1"),
        ],
    ));
    // `RGL-FR-004` (ADR-0122): the override's warning names the metrics
    // listener it applies to, not only the wire one.
    assert!(
        server
            .before_banner
            .contains("WARNING: metrics listening on")
            && server
                .before_banner
                .contains("SERVER_ALLOW_INSECURE=1 is set"),
        "the override did not warn about the metrics listener: {}",
        server.before_banner
    );
    assert!(TcpStream::connect(server.addr).is_ok());
    drop(server);
}

#[test]
fn allow_insecure_turns_the_refusal_into_a_warning_and_the_server_serves() {
    let server = spawn_listening(memory_server(
        "0.0.0.0:0",
        &[("SERVER_ALLOW_INSECURE", "1")],
    ));
    assert!(
        server.before_banner.contains("WARNING: listening on")
            && server
                .before_banner
                .contains("SERVER_ALLOW_INSECURE=1 is set"),
        "the override did not print its warning before the listening banner: {}",
        server.before_banner
    );
    assert!(
        !server.before_banner.contains("refusing to listen on"),
        "the override still refused: {}",
        server.before_banner
    );
    assert!(
        TcpStream::connect(("127.0.0.1", server.addr.port())).is_ok(),
        "the server serves on the port the banner named"
    );
    drop(server);
}
