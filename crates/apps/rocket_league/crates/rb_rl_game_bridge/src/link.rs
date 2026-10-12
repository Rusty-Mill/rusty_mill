//! The seam to the game: [`GameLink`], the loop that uses it, and the listener the game dials.
//!
//! RLBotCS keeps this seam inside the closed-source `Bridge.dll` (`TcpMessenger` and the
//! `Bridge.Models.*` message types). Here the seam is a trait, so the rules in [`crate::tick`]
//! and the loop below are tested against [`SimGameLink`](crate::SimGameLink) now, and a link
//! for the real game's socket format can be added behind it without touching them.

use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use rb_rlbot_wire::{ControllerState, GamePacket, Physics};

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
        let advanced = self
            .tracker
            .observe(clump.packet.match_info.seconds_elapsed);
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
