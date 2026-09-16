//! Minimal server-mode deployment: bind a listener, optionally confine the
//! process with `platform::security::Sandbox`, then drive
//! `RdpTransport::accept()` per connection.
//!
//! ```sh
//! cargo run --example serve --features serve-example -- 127.0.0.1:3389
//! ```
//!
//! `RdpTransport::accept()` (`src/net/server.rs`) is this crate's one entry
//! point that processes fully untrusted, attacker-controlled wire data end to
//! end — negotiation, GCC, the Security Exchange PDU, Client Info. A server
//! built on it is exactly the shape that benefits from confinement: once the
//! listening socket is bound, `Sandbox::block_inet_sockets()` denies opening
//! any *new* outbound socket from a compromised parse path (the existing
//! connections keep working — this is "no new raw sockets," not a kill
//! switch), and `Sandbox::confine_filesystem()` denies filesystem access
//! outside whatever roots the server actually needs (this example needs
//! none, so it confines to nothing at all).
//!
//! Confinement is Linux-only (Landlock + seccomp-BPF), and
//! `block_inet_sockets` further requires `x86_64` (an architecture check in
//! the seccomp filter itself). `Sandbox` reports a three-way
//! `SandboxStatus` — `Enforced`/`NotEnforced`/`Unsupported` — rather than
//! silently degrading, so this example prints whichever of the three it
//! actually got instead of assuming `Enforced`. On every other platform (or
//! non-`x86_64` Linux) it prints that confinement isn't available and runs
//! unconfined.
//!
//! This example speaks only unencrypted standard RDP security
//! (`AcceptConfig::encryption` left `None`) and does nothing with accepted
//! connections beyond printing what `accept()` negotiated before closing
//! them — it exists to demonstrate the confinement pattern around `accept`,
//! not to be a usable RDP server. Do not point this at an untrusted network
//! as-is: see `crate::security`'s note on unencrypted standard RDP security.

use std::net::TcpListener;
use std::process::ExitCode;

use rusty_rdp::net::{AcceptConfig, RdpTransport};

const WIDTH: u16 = 1024;
const HEIGHT: u16 = 768;

fn main() -> ExitCode {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:3389".to_string());

    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: failed to bind {addr}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("listening on {addr}");

    // Confine *after* binding the listening socket (accept() on an
    // already-open listener isn't a "new socket") but *before* accepting
    // any connection, so every byte of untrusted wire data is parsed under
    // confinement.
    apply_sandbox();

    let config = AcceptConfig::new(WIDTH, HEIGHT);
    loop {
        let (stream, peer) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("error: accept failed: {e}");
                continue;
            }
        };
        println!("connection from {peer}");
        let mut transport = RdpTransport::new(stream);
        match transport.accept(&config) {
            Ok(client) => {
                println!(
                    "  accepted: user_id={} io_channel={} share_id={:#010x} domain={:?} username={:?}",
                    client.user_id,
                    client.io_channel,
                    client.share_id,
                    client.client_info.domain,
                    client.client_info.username,
                );
            }
            Err(e) => eprintln!("  connection sequence failed: {e}"),
        }
    }
}

/// Apply `Sandbox` confinement and print the honest `SandboxStatus` for
/// each call. Linux-only; every other platform prints why and moves on.
#[cfg(target_os = "linux")]
fn apply_sandbox() {
    use platform::security::{Sandbox, SandboxStatus};
    use platform_linux::LinuxSandbox;

    fn report(what: &str, result: Result<SandboxStatus, platform::error::PlatformError>) {
        match result {
            Ok(SandboxStatus::Enforced) => println!("sandbox: {what} enforced"),
            Ok(SandboxStatus::NotEnforced) => {
                println!(
                    "sandbox: {what} NOT enforced (kernel/arch lacks support) — running without it"
                )
            }
            Ok(SandboxStatus::Unsupported) => {
                println!("sandbox: {what} unsupported on this backend — running without it")
            }
            Err(e) => println!("sandbox: {what} failed ({e}) — running without it"),
        }
    }

    let sandbox = LinuxSandbox;
    // No outbound connections needed once the listener is bound — deny all
    // new socket creation from here on.
    report("block_inet_sockets", sandbox.block_inet_sockets());
    // A pure protocol server touches no files at all.
    report("confine_filesystem", sandbox.confine_filesystem(&[], &[]));
}

#[cfg(not(target_os = "linux"))]
fn apply_sandbox() {
    println!("sandbox: confinement is Linux-only (Landlock/seccomp) — running unconfined");
}
