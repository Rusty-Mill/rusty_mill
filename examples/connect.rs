//! Connect to an RDP server and establish a full standard-RDP session.
//!
//! ```sh
//! cargo run --example connect -- 192.0.2.10:3389 CORP alice s3cret
//! ```
//!
//! Arguments: `host:port` (default `127.0.0.1:3389`), then optional `domain`,
//! `username`, and `password`. The example drives the whole standard-RDP
//! connection sequence — negotiation, MCS connect, channel setup, the RSA
//! security exchange, the encrypted Client Info PDU, licensing, the capability
//! exchange, and connection finalization — and reports the active session.
//!
//! Standard RDP security only: a server that requires TLS/CredSSP (most modern
//! Windows hosts) will reject the RDP-only negotiation, and the example prints
//! that. An `xrdp` server configured for RDP security is the realistic target.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;

use rusty_rdp::net::{EstablishConfig, RdpTransport};
use rusty_rdp::security::RANDOM_LEN;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:3389".to_string());
    let domain = args.next().unwrap_or_default();
    let username = args.next().unwrap_or_else(|| "user".to_string());
    let password = args.next().unwrap_or_default();

    match run(&addr, &domain, &username, &password) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(addr: &str, domain: &str, username: &str, password: &str) -> std::io::Result<()> {
    println!("connecting to {addr} ...");
    let stream = TcpStream::connect(addr)?;
    let mut rdp = RdpTransport::new(stream);

    let config = EstablishConfig::new(1024, 768, domain, username, password);
    let client_random = client_random();

    let session = rdp.establish(&config, &client_random)?;
    println!("session active:");
    println!("  MCS user id     : {}", session.user_id);
    println!("  I/O channel     : {}", session.io_channel);
    println!("  share id        : {:#010x}", session.share_id);
    println!("  server channel  : {}", session.server_channel);
    println!("handshake complete — the session is ready for input and updates.");
    Ok(())
}

/// Generate a 32-byte client random.
///
/// Uses the OS CSPRNG via `/dev/urandom` where available; falls back to a
/// fixed value elsewhere (insecure, for demonstration only).
fn client_random() -> [u8; RANDOM_LEN] {
    let mut buf = [0u8; RANDOM_LEN];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut buf).is_ok() {
            return buf;
        }
    }
    // Deterministic fallback: NOT secure, only so the example runs anywhere.
    for (i, b) in buf.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37).wrapping_add(11);
    }
    let _ = std::io::stderr().write_all(b"warning: using an insecure fixed client random\n");
    buf
}
