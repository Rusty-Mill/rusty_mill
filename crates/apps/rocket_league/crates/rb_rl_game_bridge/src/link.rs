//! The seam to the game: [`GameLink`], the loop that uses it, and the listener the game dials.
//!
//! RLBotCS keeps this seam inside the closed-source `Bridge.dll` (`TcpMessenger` and the
//! `Bridge.Models.*` message types). Here the seam is a trait, so the rules in [`crate::tick`]
//! and the loop below run over either [`SimGameLink`](crate::SimGameLink) (the game is `rb_env`)
//! or [`RocketLeagueLink`](crate::RocketLeagueLink) (the real game's socket).

use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use rb_rlbot_wire::{ControllerState, GamePacket, Physics};

use crate::game_wire::{decode_game_packet, BotSkill, Loadout};

use crate::tick::{should_distribute, Clump, Roster, TickTracker};

/// What the bridge can ask of the game.
#[derive(Debug, Clone, PartialEq)]
pub enum GameCommand {
    /// Hold `state` as the input of car `player_index` (RLBotCS maps the index to the game's
    /// actor id first, `Input.cs:11-17`).
    PlayerInput {
        player_index: u32,
        state: ControllerState,
    },
    /// Run a console command in the game.
    Console(String),
    /// Place car `player_index` (all of its physics, as the game takes them whole).
    SetCarPhysics { player_index: u32, physics: Physics },
    /// Place the ball.
    SetBallPhysics { physics: Physics },
    /// Set car `player_index`'s boost, 0 to 100.
    SetBoost { player_index: u32, amount: u8 },
    /// Spawn a bot's car for player `player_index`. The car that answers is tied to that index
    /// by the command's id, whatever order cars appear in.
    SpawnBot {
        player_index: u32,
        name: String,
        skill: BotSkill,
        team: u8,
        loadout: Loadout,
    },
}

/// A connection to a game, real or simulated.
pub trait GameLink {
    /// The next message from the game, blocking; `Ok(None)` once the game has gone away.
    fn next_clump(&mut self) -> io::Result<Option<Clump>>;

    fn send(&mut self, command: GameCommand) -> io::Result<()>;
}

/// What the loop did with one message.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// The snapshot to hand to the bots, if this message earned a distribution.
    pub distribute: Option<GamePacket>,
    /// The message announced a newly loaded map.
    pub map_loaded: bool,
}

/// `HandleServer` without the sockets: tracks time, decides what is distributed, and notices
/// map loads.
#[derive(Debug, Default)]
pub struct BridgeLoop {
    roster: Option<Roster>,
    tracker: TickTracker,
}

impl BridgeLoop {
    pub fn new() -> BridgeLoop {
        BridgeLoop::default()
    }

    /// Sets (or clears) who the match is waiting for.
    pub fn set_roster(&mut self, roster: Option<Roster>) {
        self.roster = roster;
    }

    pub fn tracker(&self) -> &TickTracker {
        &self.tracker
    }

    /// Handles one message. Order follows `BridgeHandler.cs:84-144`: time is tracked and the
    /// distribution decided from the message as it arrived; only then is a map load noted.
    /// Not ported: forcing a loaded map's phase to `Paused` in the kept state (a LAN match never
    /// reports it) and releasing queued spawn commands two messages after a load; both act on
    /// state the real game link owns.
    pub fn on_clump(&mut self, clump: Clump) -> Step {
        let advanced = self.tracker.observe(
            clump.packet.match_info.seconds_elapsed,
            clump.packet.match_info.frame_num,
        );
        let distribute = should_distribute(
            self.roster,
            &clump.packet,
            advanced,
            self.tracker.ticks_skipped(),
        )
        .then(|| clump.packet.clone());
        if clump.map_just_loaded {
            self.tracker.map_loaded();
        }
        Step {
            distribute,
            map_loaded: clump.map_just_loaded,
        }
    }

    /// Reads `link` until the game goes away, giving each distributed snapshot to `sink`.
    pub fn run(
        &mut self,
        link: &mut impl GameLink,
        mut sink: impl FnMut(GamePacket),
    ) -> io::Result<()> {
        while let Some(clump) = link.next_clump()? {
            if let Some(packet) = self.on_clump(clump).distribute {
                sink(packet);
            }
        }
        Ok(())
    }
}

/// Connections held at once while looking for the game; past it the oldest is closed.
pub const MAX_CANDIDATES: usize = 8;

/// Connections taken from the backlog per pass, so a stream of arrivals cannot starve the
/// checks on the ones already waiting, or the timeout.
const MAX_ACCEPTS_PER_PASS: usize = 8;

/// How long a connection may sit on part of a first message, with no more of it arriving,
/// before it is dropped. The game writes whole messages at once; a peer that sent a few bytes
/// and went quiet (or closed behind them: `peek` cannot see an end of stream past buffered
/// bytes) is not it. A connection with *no* bytes is not subject to this: the game's first
/// message can come a while after it dials.
pub const STALL_AFTER: Duration = Duration::from_secs(10);

/// A connection that has yet to prove it is the game.
struct Candidate {
    stream: TcpStream,
    peer: SocketAddr,
    /// Bytes waiting on the socket when it was last looked at, and when that count last grew.
    seen: usize,
    grew_at: Instant,
}

/// What a connection has said so far.
enum FirstMessage {
    /// Nothing complete yet, with this many bytes waiting.
    Waiting(usize),
    /// A whole first message, and it decodes as the game's.
    Game,
    /// Closed, or a first message that is not the game's.
    NotTheGame(String),
}

/// Looks at a non-blocking connection's first message without reading it. `buf` is scratch
/// space of at least a whole frame (`2 + u16::MAX` bytes).
fn first_message(stream: &TcpStream, buf: &mut [u8]) -> FirstMessage {
    let n = match stream.peek(buf) {
        Ok(0) => return FirstMessage::NotTheGame("it closed without a word".into()),
        Ok(n) => n,
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return FirstMessage::Waiting(0),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => return FirstMessage::Waiting(0),
        Err(e) => return FirstMessage::NotTheGame(format!("it failed: {e}")),
    };
    if n < 2 {
        return FirstMessage::Waiting(n);
    }
    let len = usize::from(u16::from_be_bytes([buf[0], buf[1]]));
    if n < 2 + len {
        return FirstMessage::Waiting(n);
    }
    match decode_game_packet(&buf[2..2 + len]) {
        Ok(_) => FirstMessage::Game,
        Err(e) => FirstMessage::NotTheGame(format!("its first message is not the game's: {e}")),
    }
}

/// Looks at every waiting connection, oldest first: the first that has spoken as the game is
/// returned (made blocking again, its first message unread); closed, junk and stalled ones are
/// dropped, with arrival order kept for the rest.
fn sift(
    candidates: &mut Vec<Candidate>,
    stall: Duration,
    buf: &mut [u8],
    note: &mut impl FnMut(&str),
) -> io::Result<Option<TcpStream>> {
    let now = Instant::now();
    let mut index = 0;
    while index < candidates.len() {
        let drop_because = match first_message(&candidates[index].stream, buf) {
            FirstMessage::Game => {
                let game = candidates.remove(index).stream;
                game.set_nonblocking(false)?;
                return Ok(Some(game));
            }
            FirstMessage::Waiting(n) => {
                let candidate = &mut candidates[index];
                if n != candidate.seen {
                    candidate.seen = n;
                    candidate.grew_at = now;
                }
                (n > 0 && now.duration_since(candidate.grew_at) >= stall)
                    .then(|| format!("it stalled part-way through a message ({n} bytes)"))
            }
            FirstMessage::NotTheGame(why) => Some(why),
        };
        match drop_because {
            Some(why) => {
                let gone = candidates.remove(index);
                note(&format!("dropped the connection from {}: {why}", gone.peer));
            }
            None => index += 1,
        }
    }
    Ok(None)
}

/// Takes a new connection in. Everyone already waiting is looked at first, so a game that has
/// spoken is found and never the one dropped to make room; then, if the places are full, the
/// oldest (in arrival order) makes way.
fn admit(
    candidates: &mut Vec<Candidate>,
    stream: TcpStream,
    peer: SocketAddr,
    stall: Duration,
    buf: &mut [u8],
    note: &mut impl FnMut(&str),
) -> io::Result<Option<TcpStream>> {
    if let Some(game) = sift(candidates, stall, buf, note)? {
        return Ok(Some(game));
    }
    if candidates.len() == MAX_CANDIDATES {
        let oldest = candidates.remove(0);
        note(&format!(
            "dropped the oldest waiting connection, from {}: too many",
            oldest.peer
        ));
    }
    note(&format!("a connection from {peer} is waiting to speak"));
    candidates.push(Candidate {
        stream,
        peer,
        seen: 0,
        grew_at: Instant::now(),
    });
    Ok(None)
}

/// The socket the game connects out to (`TcpMessenger.WaitForConnectionAsync`).
#[derive(Debug)]
pub struct GameListener {
    listener: TcpListener,
}

impl GameListener {
    /// Listens on `127.0.0.1:port`, the address [`crate::rlbot_args`] tells the game to use.
    pub fn bind(port: u16) -> io::Result<GameListener> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        Ok(GameListener { listener })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Waits for the game, telling it from anything else that dials the port: a connection is
    /// the game only once its first message arrives and decodes as one of the game's.
    ///
    /// - A connection that closes, sends something that is not a game message, or stalls
    ///   part-way through one ([`STALL_AFTER`]) is dropped.
    /// - A silent connection waits, because the game's first message can come a while after
    ///   it dials (about 45 s later, at the splash screen, in a live run).
    /// - At most [`MAX_CANDIDATES`] wait; a newer arrival drops the oldest, in arrival order,
    ///   after every waiting connection has been looked at, so a game already speaking is
    ///   never the one dropped.
    /// - Each drop and arrival is reported to `note`.
    ///
    /// The stream returned has not had a byte read from it, so the first message is still
    /// there for the link. `timeout` runs only while no connection is waiting; `Ok(None)` if
    /// it ran out. Callers that need to be told the game has dialled, not spoken, use `note`.
    pub fn wait_for_game(
        &self,
        timeout: Duration,
        note: impl FnMut(&str),
    ) -> io::Result<Option<TcpStream>> {
        self.wait_for_game_stalling(timeout, STALL_AFTER, note)
    }

    /// [`wait_for_game`](Self::wait_for_game) with the stall limit given.
    pub fn wait_for_game_stalling(
        &self,
        timeout: Duration,
        stall: Duration,
        mut note: impl FnMut(&str),
    ) -> io::Result<Option<TcpStream>> {
        let mut buf = vec![0u8; 2 + usize::from(u16::MAX)];
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut idle_since = Instant::now();
        loop {
            for _ in 0..MAX_ACCEPTS_PER_PASS {
                match self.listener.accept() {
                    Ok((stream, peer)) => {
                        stream.set_nonblocking(true)?;
                        stream.set_nodelay(true)?;
                        if let Some(game) =
                            admit(&mut candidates, stream, peer, stall, &mut buf, &mut note)?
                        {
                            return Ok(Some(game));
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            if let Some(game) = sift(&mut candidates, stall, &mut buf, &mut note)? {
                return Ok(Some(game));
            }
            let now = Instant::now();
            if !candidates.is_empty() {
                idle_since = now;
            } else if now.duration_since(idle_since) >= timeout {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits up to `timeout` for the game to connect; `Ok(None)` if it did not.
    pub fn wait_for_connection(&self, timeout: Duration) -> io::Result<Option<TcpStream>> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    stream.set_nodelay(true)?;
                    return Ok(Some(stream));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod finder_tests {
    use super::*;
    use std::io::Write;

    /// A connected pair: the listener's side (what the finder holds) and the dialler's.
    fn pair(listener: &TcpListener) -> (TcpStream, SocketAddr, TcpStream) {
        let dialler = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (held, peer) = listener.accept().unwrap();
        held.set_nonblocking(true).unwrap();
        (held, peer, dialler)
    }

    /// A whole first message as the game sends it.
    fn game_frame() -> Vec<u8> {
        use crate::game_wire::{encode_game_packet, GameBundle, GameEvent};
        let bundle = GameBundle {
            app_seconds: 0.0,
            delta_physics_frames: 1,
            events: vec![GameEvent::GameSpawn {
                map_name: "m".into(),
                mode: 1,
                team_size: 1,
                gravity: None,
            }],
        };
        rb_rlbot_wire::frame::frame(&encode_game_packet(&bundle).unwrap()).unwrap()
    }

    #[test]
    fn a_full_house_looks_at_everyone_before_dropping_the_oldest() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut buf = vec![0u8; 2 + usize::from(u16::MAX)];
        let mut candidates = Vec::new();
        let mut keep = Vec::new();
        // The game is the oldest of eight waiting, and its whole first message has arrived
        // since it was last looked at.
        let (held, peer, mut game) = pair(&listener);
        game.write_all(&game_frame()).unwrap();
        candidates.push(Candidate {
            stream: held,
            peer,
            seen: 0,
            grew_at: Instant::now(),
        });
        for _ in 1..MAX_CANDIDATES {
            let (held, peer, dialler) = pair(&listener);
            keep.push(dialler);
            candidates.push(Candidate {
                stream: held,
                peer,
                seen: 0,
                grew_at: Instant::now(),
            });
        }
        // A ninth arrives: the game is found, not dropped to make room.
        let (ninth, ninth_peer, _dialler) = pair(&listener);
        let found = admit(
            &mut candidates,
            ninth,
            ninth_peer,
            STALL_AFTER,
            &mut buf,
            &mut |_| {},
        )
        .unwrap();
        assert!(found.is_some(), "the game was dropped to make room");
    }

    #[test]
    fn rejecting_a_peer_keeps_arrival_order_for_the_next_overflow() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut buf = vec![0u8; 2 + usize::from(u16::MAX)];
        let mut candidates = Vec::new();
        let mut keep = Vec::new();
        let mut waiting = |candidates: &mut Vec<Candidate>, junk: bool| {
            let (held, peer, mut dialler) = pair(&listener);
            if junk {
                dialler
                    .write_all(&rb_rlbot_wire::frame::frame(&[0xFF; 40]).unwrap())
                    .unwrap();
            }
            keep.push(dialler);
            candidates.push(Candidate {
                stream: held,
                peer,
                seen: 0,
                grew_at: Instant::now(),
            });
            peer
        };
        // Eight waiting, oldest first: the first of them is junk, which will be rejected.
        waiting(&mut candidates, true);
        let quiet: Vec<SocketAddr> = (1..MAX_CANDIDATES)
            .map(|_| waiting(&mut candidates, false))
            .collect();
        let mut notes = Vec::new();
        // Two arrivals: the first finds the junk and rejects it (room for itself, no eviction);
        // the second finds the places full and drops the oldest *remaining*.
        for _ in 0..2 {
            let (ninth, ninth_peer, dialler) = pair(&listener);
            keep.push(dialler);
            let found = admit(
                &mut candidates,
                ninth,
                ninth_peer,
                STALL_AFTER,
                &mut buf,
                &mut |n: &str| notes.push(n.to_string()),
            )
            .unwrap();
            assert!(found.is_none());
        }
        let evicted: Vec<&String> = notes
            .iter()
            .filter(|n| n.contains("dropped the oldest"))
            .collect();
        assert_eq!(evicted.len(), 1, "{notes:?}");
        assert!(
            evicted[0].contains(&quiet[0].to_string()),
            "the oldest remaining is {}: {notes:?}",
            quiet[0]
        );
    }
}
