//! Port of `src/tcp` — the croc relay server and its client handshake.
//!
//! Protocol (identical to croc v10, so stock Go croc clients interoperate):
//! 1. Client and relay run a PAKE over the `siec` curve with the fixed weak
//!    key `[1,2,3]`, yielding a per-connection strong key. (A client may
//!    instead send the literal frame `ping`, answered with `pong`.)
//! 2. Client sends an 8-byte salt; both sides derive an AES key via
//!    PBKDF2(strong key, salt).
//! 3. Client sends the encrypted relay password; relay answers with
//!    `banner|||<client-ip>` (banner lists the extra transfer ports).
//! 4. Client sends the encrypted room name. First occupant waits (the relay
//!    sends a framed `[1]` keep-alive each second); when the second occupant
//!    arrives the relay staples the two sockets and pipes raw bytes.

use crate::comm::Comm;
use crate::crypt;
use crate::models;
use crate::pake::Pake;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_ROOM_CLEANUP_INTERVAL: Duration = Duration::from_secs(10 * 60);
pub const DEFAULT_ROOM_TTL: Duration = Duration::from_secs(3 * 60 * 60);

/// Matches the fixed weak PAKE key in Go's `tcp.clientCommunication`.
const WEAK_KEY: &[u8] = &[1, 2, 3];
const PING_ROOM: &str = "pinglkasjdlfjsaldjf";

struct Room {
    first: Option<Comm>,
    opened: Instant,
    full: bool,
}

type Rooms = Arc<Mutex<HashMap<String, Room>>>;

pub struct RelayServer {
    host: String,
    port: String,
    password: String,
    banner: String,
    rooms: Rooms,
}

impl RelayServer {
    pub fn new(host: &str, port: &str, password: &str, banner: &str) -> Self {
        RelayServer {
            host: host.to_string(),
            port: port.to_string(),
            password: password.to_string(),
            banner: banner.to_string(),
            rooms: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Bind and serve forever (mirrors `tcp.Run`). Blocks the calling thread.
    pub fn run(&self) -> std::io::Result<()> {
        let host = if self.host.is_empty() {
            "0.0.0.0"
        } else {
            &self.host
        };
        let listener = TcpListener::bind(format!("{host}:{}", self.port))?;
        log::info!("starting TCP server on {host}:{}", self.port);

        // Room janitor, mirroring deleteOldRooms.
        {
            let rooms = Arc::clone(&self.rooms);
            std::thread::spawn(move || loop {
                std::thread::sleep(DEFAULT_ROOM_CLEANUP_INTERVAL);
                let mut map = rooms.lock().unwrap();
                map.retain(|name, room| {
                    let keep = room.opened.elapsed() <= DEFAULT_ROOM_TTL;
                    if !keep {
                        log::debug!("room cleaned up: {name}");
                        if let Some(c) = room.first.take() {
                            let _ = c.stream().shutdown(Shutdown::Both);
                        }
                    }
                    keep
                });
            });
        }

        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    log::debug!("problem accepting connection: {e}");
                    continue;
                }
            };
            log::debug!("client {:?} connected", stream.peer_addr());
            let rooms = Arc::clone(&self.rooms);
            let password = self.password.clone();
            let banner = self.banner.clone();
            std::thread::spawn(move || {
                let comm = match Comm::new(stream) {
                    Ok(c) => c,
                    Err(e) => {
                        log::debug!("comm setup failed: {e}");
                        return;
                    }
                };
                if let Err(e) = client_communication(comm, &rooms, &password, &banner) {
                    log::debug!("relay client error: {e}");
                }
            });
        }
        Ok(())
    }
}

fn delete_room(rooms: &Rooms, name: &str) {
    let mut map = rooms.lock().unwrap();
    if let Some(mut room) = map.remove(name) {
        if let Some(c) = room.first.take() {
            let _ = c.stream().shutdown(Shutdown::Both);
        }
        log::debug!("room deleted: {name}");
    }
}

fn client_communication(
    mut c: Comm,
    rooms: &Rooms,
    password: &str,
    banner: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Step 1: PAKE with the fixed weak key (or a bare ping).
    let mut pake = Pake::init_curve(WEAK_KEY, 1, "siec")?;
    let a_bytes = c.receive()?;
    if a_bytes == b"ping" {
        log::debug!("sending back pong (room {PING_ROOM})");
        c.send(b"pong")?;
        return Ok(());
    }
    pake.update(&a_bytes)?;
    c.send(&pake.bytes())?;
    let strong_key = pake.session_key()?;

    // Step 2: salt → session encryption key.
    let salt = c.receive()?;
    let (key, _) = crypt::new_key(&strong_key, Some(&salt))?;

    // Step 3: password check.
    let password_bytes = crypt::decrypt(&c.receive()?, &key)?;
    if String::from_utf8_lossy(&password_bytes).trim() != password.trim() {
        let enc = crypt::encrypt(b"bad password", &key)?;
        c.send(&enc)?;
        return Err("bad password".into());
    }
    let banner = if banner.is_empty() { "ok" } else { banner };
    let peer = c
        .stream()
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let ok_msg = crypt::encrypt(format!("{banner}|||{peer}").as_bytes(), &key)?;
    c.send(&ok_msg)?;

    // Step 4: room join.
    let room = String::from_utf8(crypt::decrypt(&c.receive()?, &key)?)?;

    {
        let mut map = rooms.lock().unwrap();
        if !map.contains_key(&room) {
            map.insert(
                room.clone(),
                Room {
                    first: Some(c.try_clone()?),
                    opened: Instant::now(),
                    full: false,
                },
            );
            drop(map);
            let ok = crypt::encrypt(b"ok", &key)?;
            c.send(&ok)?;
            log::debug!("room {room} has 1");
            return first_client_keepalive(c, rooms, &room);
        }
        if map.get(&room).map(|r| r.full).unwrap_or(false) {
            drop(map);
            let full = crypt::encrypt(b"room full", &key)?;
            c.send(&full)?;
            return Ok(());
        }
        log::debug!("room {room} has 2");
        // Take the first occupant's socket and mark the room stapled while
        // still holding the lock, so the keep-alive loop can never write a
        // stray [1] frame into the piped stream.
        let room_entry = map.get_mut(&room).unwrap();
        room_entry.full = true;
        let first = room_entry.first.take().unwrap();
        drop(map);

        // Staple the two sockets (pipes first, then the "ok", same order as Go).
        let piping = start_pipe(first.try_clone()?, c.try_clone()?);
        let ok = crypt::encrypt(b"ok", &key)?;
        c.send(&ok)?;
        piping.join().ok();
        delete_room(rooms, &room);
    }
    Ok(())
}

/// While a room has one occupant, send a framed `[1]` every second so the
/// waiting client notices a dead relay; stop as soon as the room is stapled.
fn first_client_keepalive(
    mut c: Comm,
    rooms: &Rooms,
    room: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        {
            let map = rooms.lock().unwrap();
            match map.get(room) {
                None => {
                    log::debug!("room {room} is gone");
                    return Ok(());
                }
                Some(entry) => {
                    if entry.full {
                        // Stapled: the pairing thread owns both sockets now.
                        return Ok(());
                    }
                    // Send the keep-alive while holding the lock (matches the
                    // Go locking discipline; see comment at the staple site).
                    if c.send(&[1]).is_err() {
                        drop(map);
                        delete_room(rooms, room);
                        return Ok(());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Full-duplex raw byte piping between two stapled sockets. Returns a handle
/// that resolves when either direction closes (both sockets are then shut).
fn start_pipe(a: Comm, b: Comm) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let a2 = a.try_clone();
        let b2 = b.try_clone();
        let one_way = |from: Comm, to: Comm| {
            std::thread::spawn(move || {
                let mut buf = vec![0u8; models::TCP_BUFFER_SIZE];
                loop {
                    match from.stream().read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if to.stream().write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let _ = from.stream().shutdown(Shutdown::Both);
                let _ = to.stream().shutdown(Shutdown::Both);
            })
        };
        match (a2, b2) {
            (Ok(a2), Ok(b2)) => {
                let t1 = one_way(a, b2);
                let t2 = one_way(b, a2);
                let _ = t1.join();
                let _ = t2.join();
            }
            _ => {
                let _ = a.stream().shutdown(Shutdown::Both);
                let _ = b.stream().shutdown(Shutdown::Both);
            }
        }
        log::debug!("done piping");
    })
}

/// Mirrors `tcp.PingServer`: send `ping`, expect `pong`.
pub fn ping_server(address: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut c = Comm::new_connection(address, Some(Duration::from_millis(300)))?;
    c.send(b"ping")?;
    let b = c.receive()?;
    if b == b"pong" {
        Ok(())
    } else {
        Err("no pong".into())
    }
}

/// Client side of the relay handshake — mirrors `tcp.ConnectToTCPServer`.
/// Returns the connected `Comm`, the relay banner, and this client's external
/// address as seen by the relay.
pub fn connect_to_tcp_server(
    address: &str,
    password: &str,
    room: &str,
    time_limit: Option<Duration>,
) -> Result<(Comm, String, String), Box<dyn std::error::Error>> {
    let mut c = Comm::new_connection(address, time_limit)?;

    let mut pake = Pake::init_curve(WEAK_KEY, 0, "siec")?;
    c.send(&pake.bytes())?;
    let b_bytes = c.receive()?;
    pake.update(&b_bytes)?;
    let strong_key = pake.session_key()?;

    let (key, salt) = crypt::new_key(&strong_key, None)?;
    c.send(&salt)?;

    c.send(&crypt::encrypt(password.as_bytes(), &key)?)?;
    let data = crypt::decrypt(&c.receive()?, &key)?;
    let data = String::from_utf8(data)?;
    let (banner, ipaddr) = data
        .split_once("|||")
        .ok_or_else(|| format!("bad response: {data}"))?;

    c.send(&crypt::encrypt(room.as_bytes(), &key)?)?;
    let confirm = crypt::decrypt(&c.receive()?, &key)?;
    if confirm != b"ok" {
        return Err(format!("got bad response: {}", String::from_utf8_lossy(&confirm)).into());
    }
    Ok((c, banner.to_string(), ipaddr.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_relay(port: u16, password: &'static str) {
        std::thread::spawn(move || {
            RelayServer::new("127.0.0.1", &port.to_string(), password, "9998,9999")
                .run()
                .unwrap();
        });
        // Wait for the listener to come up.
        for _ in 0..50 {
            if ping_server(&format!("127.0.0.1:{port}")).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("relay did not start");
    }

    #[test]
    fn ping_pong() {
        spawn_relay(28781, "pass123");
        ping_server("127.0.0.1:28781").unwrap();
    }

    #[test]
    fn bad_password_rejected() {
        spawn_relay(28782, "pass123");
        let err = connect_to_tcp_server("127.0.0.1:28782", "wrong", "room1", None);
        assert!(err.is_err());
    }

    #[test]
    fn two_clients_pipe_data() {
        spawn_relay(28783, "pass123");
        let (mut c1, banner, ip1) =
            connect_to_tcp_server("127.0.0.1:28783", "pass123", "testroom", None).unwrap();
        assert_eq!(banner, "9998,9999");
        assert!(!ip1.is_empty());

        let (mut c2, _, _) =
            connect_to_tcp_server("127.0.0.1:28783", "pass123", "testroom", None).unwrap();

        // c1 receives keep-alive [1] frames until c2 joins, then real data.
        c2.send(b"hello from second").unwrap();
        loop {
            let data = c1.receive().unwrap();
            if data == [1] {
                continue;
            }
            assert_eq!(data, b"hello from second");
            break;
        }
        // And the reverse direction.
        c1.send(b"hello from first").unwrap();
        assert_eq!(c2.receive().unwrap(), b"hello from first");
    }

    #[test]
    fn room_full() {
        spawn_relay(28784, "pass123");
        let (_c1, _, _) =
            connect_to_tcp_server("127.0.0.1:28784", "pass123", "fullroom", None).unwrap();
        let (_c2, _, _) =
            connect_to_tcp_server("127.0.0.1:28784", "pass123", "fullroom", None).unwrap();
        let third = connect_to_tcp_server("127.0.0.1:28784", "pass123", "fullroom", None);
        assert!(third.is_err());
    }
}
