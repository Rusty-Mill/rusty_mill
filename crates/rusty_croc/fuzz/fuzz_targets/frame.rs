#![no_main]
//! Fuzz the framed-message reader: feed arbitrary bytes as if they arrived on
//! a socket and assert `Comm::receive` never panics (it may error).

use libfuzzer_sys::fuzz_target;
use std::io::Write;
use std::net::{TcpListener, TcpStream};

fuzz_target!(|data: &[u8]| {
    // Drive receive() over a real loopback socket pair — the cheapest way to
    // exercise the exact read path without exposing internals.
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(_) => return,
    };
    let addr = listener.local_addr().unwrap();
    let data = data.to_vec();
    let feeder = std::thread::spawn(move || {
        if let Ok(mut s) = TcpStream::connect(addr) {
            let _ = s.write_all(&data);
        }
    });
    if let Ok((stream, _)) = listener.accept() {
        if let Ok(mut comm) = rusty_croc::comm::Comm::new(stream) {
            let _ = comm.receive();
        }
    }
    let _ = feeder.join();
});
