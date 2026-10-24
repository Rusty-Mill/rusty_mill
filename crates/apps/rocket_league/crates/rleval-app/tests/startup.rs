//! Startup regressions against the actual native-thread/std::net binary.
mod support;

use std::collections::HashSet;
use std::net::TcpListener;
use std::process::Command;
use std::sync::{mpsc, Arc, Barrier};
use std::time::{Duration, Instant};
use support::{probe, Server};

#[test]
fn concurrent_servers_report_unique_addresses_they_still_own() {
    let barrier = Arc::new(Barrier::new(8));
    let children: Vec<_> = (0..8)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                Server::start()
            })
        })
        .collect();
    // Hold every child alive while checking uniqueness and exclusive ownership.
    let servers: Vec<_> = children
        .into_iter()
        .map(|child| child.join().unwrap())
        .collect();
    let ports: HashSet<_> = servers.iter().map(|server| server.1).collect();
    assert_eq!(ports.len(), servers.len());
    for port in ports {
        assert_ne!(port, 0);
        let address = ([127, 0, 0, 1], port).into();
        assert!(
            TcpListener::bind(address).is_err(),
            "child must retain {address}"
        );
        probe(address, Instant::now() + Duration::from_secs(1)).unwrap();
    }
}

#[test]
fn early_exit_reports_status_and_final_diagnostics() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rleval"));
    command.args(["serve", "--invalid-startup-regression-flag"]);
    let error = Server::start_command(command, Duration::from_secs(30))
        .err()
        .expect("must fail");
    assert!(error.contains("child exited:"), "{error}");
    assert!(
        error.contains("unknown flag --invalid-startup-regression-flag"),
        "{error}"
    );
    assert!(
        error.contains("stdout (tail):") && error.contains("stderr (tail):"),
        "{error}"
    );
    assert!(error.contains("address: None"), "{error}");
}

#[test]
fn failed_bind_never_announces_readiness() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rleval"));
    command.args(["serve", "--host", "127.0.0.1", "--port", &port]);
    let error = Server::start_command(command, Duration::from_secs(5))
        .err()
        .expect("occupied port");
    assert!(error.contains("child exited:"), "{error}");
    assert!(error.contains("address: None"), "{error}");
    assert!(!error.contains("serving on http://"), "{error}");
    assert!(!error.contains("stderr (tail):\n\n"), "{error}");
}

#[test]
fn health_probe_times_out_when_peer_accepts_but_does_not_respond() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release, wait) = mpsc::channel::<()>();
    let peer = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        let _ = wait.recv_timeout(Duration::from_secs(5));
    });
    let result = probe(address, Instant::now() + Duration::from_millis(100));
    let _ = release.send(());
    peer.join().unwrap();
    let error = result.expect_err("a silent peer cannot make the probe hang");
    assert!(
        matches!(
            error.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ),
        "{error}"
    );
}
