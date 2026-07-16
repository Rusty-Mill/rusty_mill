//! Connect to an RDP server, establish a session, and capture the screen.
//!
//! ```sh
//! cargo run --example connect -- 192.0.2.10:3389 CORP alice s3cret
//! ```
//!
//! Arguments: `host:port` (default `127.0.0.1:3389`), then optional `domain`,
//! `username`, and `password`. The example drives the whole standard-RDP
//! connection sequence — negotiation, MCS connect, channel setup, the RSA
//! security exchange, the encrypted Client Info PDU, licensing, the capability
//! exchange, and connection finalization — then pumps server updates into a
//! framebuffer until the stream goes quiet and writes it to `screen.ppm`.
//!
//! Standard RDP security only: a server that requires TLS/CredSSP (most modern
//! Windows hosts) will reject the RDP-only negotiation, and the example prints
//! that. An `xrdp` server configured for RDP security is the realistic target.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

use rusty_rdp::display::Framebuffer;
use rusty_rdp::net::{EstablishConfig, RdpEvent, RdpTransport};
use rusty_rdp::output::PaletteEntry;
use rusty_rdp::security::RANDOM_LEN;

const WIDTH: u16 = 1024;
const HEIGHT: u16 = 768;

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

    let mut config = EstablishConfig::new(WIDTH, HEIGHT, domain, username, password);
    config.bits_per_pixel = 16;
    let client_random = client_random();

    let session = rdp.establish(&config, &client_random)?;
    println!("session active:");
    println!("  MCS user id     : {}", session.user_id);
    println!("  I/O channel     : {}", session.io_channel);
    println!("  share id        : {:#010x}", session.share_id);
    println!("  server channel  : {}", session.server_channel);

    // Stop reading once the server stops sending for a couple of seconds.
    rdp.get_ref()
        .set_read_timeout(Some(Duration::from_secs(2)))?;

    let mut framebuffer = Framebuffer::new(WIDTH as usize, HEIGHT as usize);
    let mut palette: Option<Vec<PaletteEntry>> = None;
    let mut rectangles = 0usize;

    loop {
        match rdp.recv_event() {
            Ok(RdpEvent::Bitmap(rects)) => {
                for rect in &rects {
                    framebuffer.apply_bitmap(rect, palette.as_deref()).ok();
                    rectangles += 1;
                }
            }
            Ok(RdpEvent::Palette(update)) => palette = Some(update.entries),
            Ok(RdpEvent::DeactivateAll) => {
                println!("server deactivated the share; stopping.");
                break;
            }
            Ok(_) => {} // finalization / other PDUs: ignore
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                println!("no more updates (read timed out).");
                break;
            }
            Err(e) => return Err(e),
        }
    }

    println!("applied {rectangles} bitmap rectangle(s).");
    if rectangles > 0 {
        std::fs::write("screen.ppm", framebuffer.to_ppm())?;
        println!("wrote screen.ppm ({WIDTH}x{HEIGHT}).");
    }
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
