//! Interop demo: join a room on any croc relay (Go or Rust) and print the
//! banner — exercises the full client-side relay handshake (SIEC PAKE,
//! PBKDF2, AES-GCM, framing) against a live server.
//!
//! Usage: cargo run --example join_room -- <relay-address> <password> <room>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let address = args.get(1).map(String::as_str).unwrap_or("127.0.0.1:9009");
    let password = args.get(2).map(String::as_str).unwrap_or("pass123");
    let room = args.get(3).map(String::as_str).unwrap_or("interop-room");

    let (_comm, banner, ip) = rusty_croc::tcp::connect_to_tcp_server(
        address,
        password,
        room,
        Some(std::time::Duration::from_secs(10)),
    )?;
    println!("connected to {address}: banner={banner} ip={ip}");
    Ok(())
}
