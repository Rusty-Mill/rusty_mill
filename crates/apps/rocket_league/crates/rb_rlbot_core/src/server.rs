//! The sockets: a std::net, single-threaded RLBot core around a [`MatchBackend`] (the
//! [`Simulation`](crate::Simulation), or a real game through `rb_rl_game_bridge`).
//!
//! One loop does everything, in this order each pass: accept new clients, read what the
//! clients sent (handshakes, inputs, state setting, stop), then, once every agent the match
//! waits for is ready, advance the match according to the [`Pacing`] and send the new
//! [`GamePacket`] to every ready client. No threads and no async: the work is one local socket
//! per client and a CPU-bound step, so there is no I/O concurrency to exploit.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use rb_rlbot_wire::frame::{frame, FrameDecoder};
use rb_rlbot_wire::{CoreMessage, InterfaceMessage};

use crate::backend::{MatchBackend, Poll};
use crate::sim::Simulation;

/// Seconds per physics tick, as `Duration`: the game's 120 Hz.
const TICK: Duration = Duration::from_nanos(8_333_333);

/// The most bytes read from one client in one pass of the loop, so a client that floods the
/// socket cannot starve the others or the ticks; the rest waits in the socket for the next pass.
const MAX_READ_PER_PASS: usize = 64 * 1024;

/// The most unsent bytes held for one client that is not reading (about a second of packets);
/// past this it is dropped, so a stalled client never holds up the loop or grows without bound.
const MAX_OUTBOX: usize = 1024 * 1024;

/// The most clients served at once; a connection past this is closed at once.
const MAX_CLIENTS: usize = 64;

/// The most arrivals one pass of the loop looks at, refused and aborted ones included, so a
/// stream of connections cannot keep `accept` from returning to the ticks and the bots.
const MAX_ACCEPTS_PER_PASS: usize = 64;

/// How long a connection may stay silent before it sends its connection settings.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// If real-time pacing falls this far behind (a debugger, a suspended laptop), it restarts the
/// schedule instead of stepping flat out to catch up.
const MAX_LAG: Duration = Duration::from_secs(1);

/// How the match clock relates to the wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pacing {
    /// One tick per 1/120 s of wall time, with the latest input each car has sent. What a
    /// bot sees in the real game; bots that answer late miss ticks.
    RealTime,
    /// A tick only once every ready controlling client has answered the previous packet, or
    /// `input_timeout` has passed. The match does not depend on how fast bots run, so the same
    /// bots produce the same match every time: the mode for repeatable runs.
    Lockstep { input_timeout: Duration },
}

/// Something worth telling the operator; the core logs nothing itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Listening(SocketAddr),
    ClientConnected(SocketAddr),
    AgentJoined {
        agent_id: String,
        cars: usize,
    },
    AgentReady {
        agent_id: String,
    },
    MatchStarted,
    /// A client sent input for a car it does not control.
    InputRejected {
        agent_id: String,
        player_index: u32,
    },
    /// A client sent a message that core does not act on (and why).
    Ignored {
        agent_id: String,
        what: &'static str,
    },
    /// Lockstep gave up waiting for these cars' inputs and stepped anyway.
    LockstepTimeout {
        missing_cars: Vec<u32>,
    },
    ClientDropped {
        agent_id: Option<String>,
        reason: String,
    },
}

/// Why [`Core::run`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// A client sent a stop command; `shutdown_server` is its flag.
    StopCommand { shutdown_server: bool },
    /// The caller's stop flag was set.
    Interrupted,
    /// The game behind a self-paced backend closed its connection.
    GameClosed,
}

/// What came of reading one client: a stop command it sent, and why to drop it, if to.
struct Pumped {
    stop: Option<RunEnd>,
    drop: Option<String>,
}

struct Client {
    stream: TcpStream,
    peer: SocketAddr,
    decoder: FrameDecoder,
    agent_id: Option<String>,
    /// Car indices this client may send input for.
    controllables: Vec<u32>,
    ready: bool,
    /// Cars of `controllables` that have sent input since the last packet.
    answered: Vec<u32>,
    /// Its opening reply (team, match, field) is held until the backend has a match to show.
    awaiting_opening: bool,
    connected: Instant,
    /// Bytes accepted for sending that the socket has not taken yet, ahead of any later frame.
    outbox: Vec<u8>,
}

impl Client {
    /// Whether it has sent its connection settings (the handshake's first step).
    fn introduced(&self) -> bool {
        self.agent_id.is_some() || self.awaiting_opening || self.ready
    }

    /// Queues `bytes` behind anything unsent and writes what the socket takes. A client whose
    /// backlog passes [`MAX_OUTBOX`] is an error, as is a failed write.
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.outbox.len() + bytes.len() > MAX_OUTBOX {
            return Err(io::Error::new(
                ErrorKind::WouldBlock,
                "not reading its packets",
            ));
        }
        self.outbox.extend_from_slice(bytes);
        self.flush()
    }

    /// Writes as much of the outbox as the socket takes now.
    fn flush(&mut self) -> io::Result<()> {
        while !self.outbox.is_empty() {
            match self.stream.write(&self.outbox) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn label(&self) -> String {
        self.agent_id
            .clone()
            .unwrap_or_else(|| self.peer.to_string())
    }
}

/// An RLBot core: bots on one side, a [`MatchBackend`] on the other.
pub struct Core<B = Simulation> {
    listener: TcpListener,
    backend: B,
    pacing: Pacing,
    clients: Vec<Client>,
    started: bool,
    /// An `accept` failure was reported and none has succeeded since (reported once per streak).
    accept_failing: bool,
    /// A backend failure noticed while handling a client, to end the run with.
    fatal: Option<io::Error>,
}

impl<B: MatchBackend> Core<B> {
    /// Listens on `addr` (port 0 picks a free one; see [`Core::local_addr`]). `pacing` applies
    /// to a backend that is stepped by core; a self-paced one (a real game) sets its own.
    pub fn bind(addr: impl ToSocketAddrs, backend: B, pacing: Pacing) -> io::Result<Core<B>> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Core {
            listener,
            backend,
            pacing,
            clients: Vec::new(),
            started: false,
            accept_failing: false,
            fatal: None,
        })
    }

    /// The backend, for a caller that wants to look at the match afterwards.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Serves until a client stops the match or `stop` is set, then tells every client to
    /// disconnect. One match per call: the match starts when every agent the configuration
    /// names has finished its handshake (at once when it names none).
    pub fn run(
        &mut self,
        stop: &AtomicBool,
        mut on_event: impl FnMut(Event),
    ) -> io::Result<RunEnd> {
        on_event(Event::Listening(self.local_addr()?));
        let mut clock = Clock::new();
        let end = loop {
            if stop.load(Ordering::Relaxed) {
                break RunEnd::Interrupted;
            }
            self.accept(&mut on_event)?;
            let stop_command = self.read_clients(&mut on_event);
            self.flush_outboxes(&mut on_event);
            if let Some(e) = self.fatal.take() {
                return Err(e);
            }
            if let Some(end) = stop_command {
                break end;
            }
            let mut polled_packet = false;
            if self.backend.self_paced() {
                // A game is read from the start: the match cannot begin, or the field be
                // described, until it has said what it is playing.
                match self.backend.poll()? {
                    Poll::Packet => polled_packet = true,
                    Poll::Idle => {}
                    Poll::Closed => break RunEnd::GameClosed,
                }
            }
            self.flush_openings(&mut on_event);
            if !self.started && self.backend.ready() && self.all_expected_ready() {
                self.started = true;
                on_event(Event::MatchStarted);
                self.broadcast_packet(&mut on_event);
                // An input a bot sent before the match began does not answer this first packet.
                self.clear_answers();
                clock = Clock::new();
            } else if self.started && self.backend.self_paced() {
                if polled_packet {
                    self.broadcast_packet(&mut on_event);
                    self.clear_answers();
                } else {
                    thread::sleep(Duration::from_micros(200));
                }
            } else if self.started && self.due(&mut clock, &mut on_event) {
                self.backend.step()?;
                self.broadcast_packet(&mut on_event);
                clock.advance();
                self.clear_answers();
            } else if !self.backend.self_paced() || !polled_packet {
                thread::sleep(Duration::from_micros(200));
            }
        };
        self.broadcast(&CoreMessage::DisconnectSignal, &mut on_event);
        self.close_clients();
        Ok(end)
    }

    /// Closes every client without losing the disconnect signal just sent. Dropping a socket
    /// that still has unread input from the peer makes the OS reset the connection (Windows
    /// does), and a reset discards what the peer had not yet read: a bot that answered the last
    /// packet would never see the signal. So the write side is shut down first (a clean end of
    /// stream after the signal) and what the clients still send is read and dropped for a moment.
    fn close_clients(&mut self) {
        const DRAIN: Duration = Duration::from_millis(200);
        let deadline = Instant::now() + DRAIN;
        let mut open = std::mem::take(&mut self.clients);
        let mut sink = [0u8; 1024];
        while !open.is_empty() && Instant::now() < deadline {
            open.retain_mut(|client| {
                // The signal may still be queued behind a slow reader: keep trying to send it.
                if client.flush().is_err() {
                    return false;
                }
                if client.outbox.is_empty() {
                    // A clean end of stream after the signal; a client already gone is not
                    // worth reporting at shutdown.
                    let _ = client.stream.shutdown(std::net::Shutdown::Write);
                }
                match client.stream.read(&mut sink) {
                    // The peer closed its sending side only: it may still be reading, so keep
                    // the client until what is queued for it has gone (a full close fails the
                    // flush above).
                    Ok(0) => !client.outbox.is_empty(),
                    Ok(_) => true,
                    Err(e) => e.kind() == ErrorKind::WouldBlock,
                }
            });
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn clear_answers(&mut self) {
        for client in &mut self.clients {
            client.answered.clear();
        }
    }

    fn accept(&mut self, on_event: &mut impl FnMut(Event)) -> io::Result<()> {
        for _ in 0..MAX_ACCEPTS_PER_PASS {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    self.accept_failing = false;
                    if self.clients.len() >= MAX_CLIENTS {
                        on_event(Event::ClientDropped {
                            agent_id: None,
                            reason: format!("{peer}: too many clients"),
                        });
                        continue;
                    }
                    // A connection that is already reset can fail these; it is that client's
                    // problem, not the match's.
                    if let Err(e) = stream
                        .set_nodelay(true)
                        .and_then(|()| stream.set_nonblocking(true))
                    {
                        on_event(Event::ClientDropped {
                            agent_id: None,
                            reason: format!("setting up {peer}: {e}"),
                        });
                        continue;
                    }
                    on_event(Event::ClientConnected(peer));
                    self.clients.push(Client {
                        stream,
                        peer,
                        decoder: FrameDecoder::new(),
                        agent_id: None,
                        controllables: Vec::new(),
                        ready: false,
                        answered: Vec::new(),
                        awaiting_opening: false,
                        connected: Instant::now(),
                        outbox: Vec::new(),
                    });
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                // A client that connected and vanished before it was accepted.
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset
                    ) => {}
                // Anything else (out of descriptors, say) costs that connection, not the match;
                // reported once until an accept succeeds.
                Err(e) => {
                    if !self.accept_failing {
                        self.accept_failing = true;
                        on_event(Event::ClientDropped {
                            agent_id: None,
                            reason: format!("accept: {e}"),
                        });
                    }
                    return Ok(());
                }
            }
        }
        // The budget is spent: the rest waits in the listen queue for the next pass.
        Ok(())
    }

    fn all_expected_ready(&self) -> bool {
        self.backend.expected_agents().iter().all(|id| {
            self.clients
                .iter()
                .any(|c| c.ready && c.agent_id.as_deref() == Some(id))
        })
    }

    /// Reads and handles whatever every client has sent. `Some` once a stop command arrived.
    fn read_clients(&mut self, on_event: &mut impl FnMut(Event)) -> Option<RunEnd> {
        let mut end = None;
        let mut index = 0;
        while index < self.clients.len() {
            let pumped = self.pump(index, on_event);
            // A stop counts even if its sender then closed the socket or sent something bad.
            end = end.or(pumped.stop);
            let client = &self.clients[index];
            let silent = !client.introduced() && client.connected.elapsed() > HANDSHAKE_TIMEOUT;
            let drop = pumped
                .drop
                .or_else(|| silent.then(|| "no connection settings".to_string()));
            if let Some(reason) = drop {
                let gone = self.clients.remove(index);
                on_event(Event::ClientDropped {
                    agent_id: gone.agent_id,
                    reason,
                });
                continue;
            }
            index += 1;
        }
        end
    }

    /// Reads what client `index` has sent (up to [`MAX_READ_PER_PASS`] bytes) and handles each
    /// complete message. Everything that arrived is handled before a close or an error is acted
    /// on, so a client that sends a stop (or its last input) and then exits is not ignored.
    fn pump(&mut self, index: usize, on_event: &mut impl FnMut(Event)) -> Pumped {
        let mut chunk = [0u8; 4096];
        let mut read = 0;
        let mut drop = None;
        while read < MAX_READ_PER_PASS {
            match self.clients[index].stream.read(&mut chunk) {
                Ok(0) => {
                    drop = Some("connection closed".to_string());
                    break;
                }
                Ok(n) => {
                    read += n;
                    self.clients[index].decoder.push(&chunk[..n]);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => {
                    drop = Some(format!("read: {e}"));
                    break;
                }
            }
        }
        let mut stop = None;
        while let Some(payload) = self.clients[index].decoder.next_frame() {
            let message = match InterfaceMessage::from_payload(&payload) {
                Ok(message) => message,
                Err(e) => {
                    drop = Some(format!("undecodable message: {e}"));
                    break;
                }
            };
            match self.handle(index, message, on_event) {
                Ok(Some(end)) => stop = Some(end),
                Ok(None) => {}
                Err(reason) => {
                    drop = Some(reason);
                    break;
                }
            }
        }
        Pumped { stop, drop }
    }

    fn handle(
        &mut self,
        index: usize,
        message: InterfaceMessage,
        on_event: &mut impl FnMut(Event),
    ) -> Result<Option<RunEnd>, String> {
        let label = self.clients[index].label();
        // Stopping the match, setting its state and pings are for a client that has introduced
        // itself; a bare connection can do none of them.
        if matches!(
            message,
            InterfaceMessage::StopCommand(_)
                | InterfaceMessage::DesiredGameState(_)
                | InterfaceMessage::PingRequest(_)
        ) && !self.clients[index].introduced()
        {
            return Err("sent a command before its connection settings".into());
        }
        match message {
            InterfaceMessage::ConnectionSettings(settings) => {
                self.join(index, settings.agent_id.trim(), on_event)?;
            }
            InterfaceMessage::InitComplete(_) => {
                let client = &mut self.clients[index];
                if client.agent_id.is_none() && !client.ready && !client.awaiting_opening {
                    return Err("sent InitComplete before its connection settings".into());
                }
                if !client.ready {
                    client.ready = true;
                    on_event(Event::AgentReady { agent_id: label });
                }
            }
            InterfaceMessage::PlayerInput(input) => {
                let client = &mut self.clients[index];
                if client.controllables.contains(&input.player_index) {
                    match self
                        .backend
                        .set_input(input.player_index, &input.controller_state)
                    {
                        Ok(_) => {}
                        // A game's per-frame byte budget is spent: this input is dropped, as
                        // RLBot drops it; the bot sends another next packet.
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                        Err(e) => self.fatal = Some(e),
                    }
                    if !client.answered.contains(&input.player_index) {
                        client.answered.push(input.player_index);
                    }
                } else {
                    on_event(Event::InputRejected {
                        agent_id: label,
                        player_index: input.player_index,
                    });
                }
            }
            InterfaceMessage::DesiredGameState(desired) => {
                if self.backend.configuration().enable_state_setting {
                    match self.backend.apply_desired(&desired) {
                        Err(e) if e.kind() == ErrorKind::InvalidInput => on_event(Event::Ignored {
                            agent_id: label,
                            what: "nonfinite desired state",
                        }),
                        Ok(false) => {}
                        Ok(true) => on_event(Event::Ignored {
                            agent_id: label,
                            what: "state for a car that does not exist",
                        }),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                        Err(e) => self.fatal = Some(e),
                    }
                } else {
                    on_event(Event::Ignored {
                        agent_id: label,
                        what: "state setting is disabled in the match configuration",
                    });
                }
            }
            InterfaceMessage::StopCommand(stop) => {
                return Ok(Some(RunEnd::StopCommand {
                    shutdown_server: stop.shutdown_server,
                }));
            }
            InterfaceMessage::PingResponse(_) => {}
            // The client says it is leaving: let it go without calling it an error.
            InterfaceMessage::DisconnectSignal => {
                return Err("sent a disconnect signal".into());
            }
            InterfaceMessage::MatchConfiguration(_) | InterfaceMessage::StartCommand(_) => {
                on_event(Event::Ignored {
                    agent_id: label,
                    what: "starting a new match over the socket is not supported",
                });
            }
            InterfaceMessage::PingRequest(_) => on_event(Event::Ignored {
                agent_id: label,
                what: "latency pings are not answered",
            }),
            // Render groups, comms, loadouts and the like are not modelled; skipping them is
            // what lets a client that sends them stay connected.
            InterfaceMessage::Other(_) => on_event(Event::Ignored {
                agent_id: label,
                what: "a message type this core does not model",
            }),
        }
        Ok(None)
    }

    /// Handles a client's introduction: records who it is and sends what core sends back.
    /// A named agent gets its cars, the match configuration and the field, then must send
    /// `InitComplete`; an anonymous client (a match runner or logger) gets the last two and
    /// packets straight away, and may not send input.
    fn join(
        &mut self,
        index: usize,
        agent_id: &str,
        on_event: &mut impl FnMut(Event),
    ) -> Result<(), String> {
        if self.clients[index].agent_id.is_some() || self.clients[index].ready {
            return Err("sent its connection settings twice".into());
        }
        let named = !agent_id.is_empty();
        if named
            && self
                .clients
                .iter()
                .any(|c| c.agent_id.as_deref() == Some(agent_id))
        {
            return Err(format!("agent id {agent_id:?} is already connected"));
        }
        if named {
            let team = self.backend.controllable_team_info(agent_id);
            let client = &mut self.clients[index];
            client.agent_id = Some(agent_id.to_string());
            client.controllables = team.controllables.iter().map(|c| c.index).collect();
            on_event(Event::AgentJoined {
                agent_id: agent_id.to_string(),
                cars: team.controllables.len(),
            });
        }
        self.clients[index].awaiting_opening = true;
        self.flush_opening(index)
    }

    /// Sends client `index` its opening reply if it is waiting for one and the backend has a
    /// match to describe (a game that has not yet announced its map cannot say what the field
    /// is). A named agent gets its cars, the match configuration and the field; an anonymous
    /// client gets the last two, and from then on packets.
    fn flush_opening(&mut self, index: usize) -> Result<(), String> {
        if !self.clients[index].awaiting_opening || !self.backend.ready() {
            return Ok(());
        }
        let mut opening = Vec::new();
        match self.clients[index].agent_id.clone() {
            Some(agent_id) => opening.push(CoreMessage::ControllableTeamInfo(
                self.backend.controllable_team_info(&agent_id),
            )),
            None => self.clients[index].ready = true,
        }
        opening.push(CoreMessage::MatchConfiguration(
            self.backend.configuration().clone(),
        ));
        opening.push(CoreMessage::FieldInfo(self.backend.field_info()));
        self.clients[index].awaiting_opening = false;
        let bytes = encode(&opening).map_err(|e| format!("encoding the opening: {e}"))?;
        self.clients[index]
            .send(&bytes)
            .map_err(|e| format!("write: {e}"))
    }

    /// Writes what slow clients have not yet taken; one that fails is dropped.
    fn flush_outboxes(&mut self, on_event: &mut impl FnMut(Event)) {
        let mut index = 0;
        while index < self.clients.len() {
            match self.clients[index].flush() {
                Ok(()) => index += 1,
                Err(e) => {
                    let gone = self.clients.remove(index);
                    on_event(Event::ClientDropped {
                        agent_id: gone.agent_id,
                        reason: format!("write: {e}"),
                    });
                }
            }
        }
    }

    /// Sends every waiting client its opening now that the backend may be ready.
    fn flush_openings(&mut self, on_event: &mut impl FnMut(Event)) {
        let mut index = 0;
        while index < self.clients.len() {
            match self.flush_opening(index) {
                Ok(()) => index += 1,
                Err(reason) => {
                    let gone = self.clients.remove(index);
                    on_event(Event::ClientDropped {
                        agent_id: gone.agent_id,
                        reason,
                    });
                }
            }
        }
    }

    /// Whether the next tick should run now.
    fn due(&mut self, clock: &mut Clock, on_event: &mut impl FnMut(Event)) -> bool {
        match self.pacing {
            Pacing::RealTime => clock.real_time_due(),
            Pacing::Lockstep { input_timeout } => {
                let missing: Vec<u32> = self
                    .clients
                    .iter()
                    .filter(|c| c.ready)
                    .flat_map(|c| {
                        c.controllables
                            .iter()
                            .filter(|car| !c.answered.contains(car))
                            .copied()
                    })
                    .collect();
                if missing.is_empty() {
                    return true;
                }
                if clock.since_packet() >= input_timeout {
                    on_event(Event::LockstepTimeout {
                        missing_cars: missing,
                    });
                    return true;
                }
                false
            }
        }
    }

    fn broadcast_packet(&mut self, on_event: &mut impl FnMut(Event)) {
        let packet = CoreMessage::GamePacket(self.backend.packet());
        self.broadcast(&packet, on_event);
    }

    /// Sends `message` to every ready client; one that cannot take it is dropped.
    fn broadcast(&mut self, message: &CoreMessage, on_event: &mut impl FnMut(Event)) {
        let bytes = match encode(std::slice::from_ref(message)) {
            Ok(bytes) => bytes,
            Err(e) => {
                // A packet that cannot be encoded fails for every client alike.
                for gone in self.clients.drain(..) {
                    on_event(Event::ClientDropped {
                        agent_id: gone.agent_id,
                        reason: format!("encoding a message: {e}"),
                    });
                }
                return;
            }
        };
        let mut index = 0;
        while index < self.clients.len() {
            if !self.clients[index].ready {
                index += 1;
                continue;
            }
            match self.clients[index].send(&bytes) {
                Ok(()) => index += 1,
                Err(e) => {
                    let gone = self.clients.remove(index);
                    on_event(Event::ClientDropped {
                        agent_id: gone.agent_id,
                        reason: format!("write: {e}"),
                    });
                }
            }
        }
    }
}

/// The wall-clock side of pacing.
struct Clock {
    next_tick: Instant,
    packet_sent: Instant,
}

impl Clock {
    fn new() -> Clock {
        let now = Instant::now();
        Clock {
            next_tick: now + TICK,
            packet_sent: now,
        }
    }

    fn real_time_due(&mut self) -> bool {
        let now = Instant::now();
        if now.saturating_duration_since(self.next_tick) > MAX_LAG {
            self.next_tick = now;
        }
        now >= self.next_tick
    }

    fn since_packet(&self) -> Duration {
        self.packet_sent.elapsed()
    }

    fn advance(&mut self) {
        self.next_tick += TICK;
        self.packet_sent = Instant::now();
    }
}

fn encode(messages: &[CoreMessage]) -> Result<Vec<u8>, rb_rlbot_wire::Error> {
    let mut bytes = Vec::new();
    for message in messages {
        bytes.extend_from_slice(&frame(&message.to_payload()?)?);
    }
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rb_rlbot_wire::{CustomBot, MatchConfiguration, PlayerClass, PlayerConfiguration};

    fn core() -> Core<Simulation> {
        let bot = PlayerConfiguration {
            variety: PlayerClass::CustomBot(CustomBot {
                name: "t".into(),
                root_dir: String::new(),
                run_command: String::new(),
                agent_id: "t/bot".into(),
                hivemind: false,
                environment: None,
            }),
            team: 0,
            player_id: 0,
        };
        let sim = Simulation::new(MatchConfiguration {
            player_configurations: vec![bot],
            ..MatchConfiguration::default()
        })
        .unwrap();
        Core::bind("127.0.0.1:0", sim, Pacing::RealTime).unwrap()
    }

    fn pair() -> (TcpStream, Client) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let ours = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (theirs, peer) = listener.accept().unwrap();
        theirs.set_nonblocking(true).unwrap();
        let client = Client {
            stream: theirs,
            peer,
            decoder: FrameDecoder::new(),
            agent_id: None,
            controllables: Vec::new(),
            ready: false,
            answered: Vec::new(),
            awaiting_opening: false,
            connected: Instant::now(),
            outbox: Vec::new(),
        };
        (ours, client)
    }

    #[test]
    fn a_client_that_never_reads_is_refused_without_blocking() {
        let (_silent, mut client) = pair();
        let chunk = vec![0u8; 64 * 1024];
        let started = Instant::now();
        let mut error = None;
        for _ in 0..4096 {
            if let Err(e) = client.send(&chunk) {
                error = Some(e);
                break;
            }
        }
        let error = error.expect("the backlog is bounded");
        assert_eq!(error.kind(), ErrorKind::WouldBlock);
        assert!(client.outbox.len() <= MAX_OUTBOX);
        // Nothing waited on the peer: the old blocking write took two seconds to give up.
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_slow_reader_gets_every_byte_in_order() {
        let (mut ours, mut client) = pair();
        // Several distinct frames-worth of bytes, sent until the socket is full and a backlog
        // of whole sends is queued, so partial writes and ordering across sends are exercised.
        let mut bytes = Vec::new();
        let mut round = 0u8;
        while client.outbox.len() < 256 * 1024 {
            let chunk: Vec<u8> = (0..16 * 1024u32).map(|i| (i % 251) as u8 ^ round).collect();
            client.send(&chunk).unwrap();
            bytes.extend_from_slice(&chunk);
            round = round.wrapping_add(1);
            assert!(bytes.len() < 64 * 1024 * 1024, "the socket never filled");
        }
        assert!(!client.outbox.is_empty(), "a backlog is pending");
        ours.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut got = Vec::new();
        let mut chunk = [0u8; 65536];
        while got.len() < bytes.len() {
            assert!(Instant::now() < deadline, "the backlog never drained");
            client.flush().unwrap();
            match ours.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(got, bytes);
        assert!(client.outbox.is_empty());
    }

    #[test]
    fn connections_past_the_cap_are_closed_and_the_match_is_unharmed() {
        let mut core = core();
        let addr = core.local_addr().unwrap();
        let sockets: Vec<TcpStream> = (0..MAX_CLIENTS + 3)
            .map(|_| TcpStream::connect(addr).unwrap())
            .collect();
        let mut events = Vec::new();
        // The kernel queues the connections; a pass looks at a bounded number of them.
        for _ in 0..4 {
            core.accept(&mut |e| events.push(e)).unwrap();
        }
        assert_eq!(core.clients.len(), MAX_CLIENTS);
        let refused = events
            .iter()
            .filter(
                |e| matches!(e, Event::ClientDropped { reason, .. } if reason.contains("too many")),
            )
            .count();
        assert_eq!(refused, 3);
        drop(sockets);
    }

    #[test]
    fn one_pass_looks_at_a_bounded_number_of_arrivals() {
        let mut core = core();
        let addr = core.local_addr().unwrap();
        let sockets: Vec<TcpStream> = (0..MAX_CLIENTS)
            .map(|_| TcpStream::connect(addr).unwrap())
            .collect();
        let mut events = Vec::new();
        core.accept(&mut |e| events.push(e)).unwrap();
        assert_eq!(core.clients.len(), MAX_CLIENTS);
        // Now a flood the cap refuses: one pass handles at most the budget of them, however
        // many are waiting, and returns.
        // (Fewer than the listen backlog, or the connects themselves would block.)
        let waiting = MAX_ACCEPTS_PER_PASS + 30;
        let flood: Vec<TcpStream> = (0..waiting)
            .map(|_| TcpStream::connect(addr).unwrap())
            .collect();
        events.clear();
        core.accept(&mut |e| events.push(e)).unwrap();
        assert_eq!(events.len(), MAX_ACCEPTS_PER_PASS);
        core.accept(&mut |e| events.push(e)).unwrap();
        assert_eq!(events.len(), waiting, "the rest are seen on the next pass");
        drop((sockets, flood));
    }

    #[test]
    fn a_peer_that_closed_its_write_side_still_gets_what_was_queued() {
        let mut core = core();
        let (mut ours, mut client) = pair();
        // Fill the socket until a lot is left waiting in the outbox (more than one drain pass can send).
        let chunk: Vec<u8> = (0..32 * 1024u32).map(|i| (i % 251) as u8).collect();
        let mut bytes = Vec::new();
        while client.outbox.len() < 512 * 1024 {
            client.send(&chunk).unwrap();
            bytes.extend_from_slice(&chunk);
            assert!(bytes.len() < 64 * 1024 * 1024, "the socket never filled");
        }
        // Read EOF on the core's side, while the peer still reads.
        ours.shutdown(std::net::Shutdown::Write).unwrap();
        core.clients.push(client);
        let reader = thread::spawn(move || {
            // Start reading only after the first drain pass has met the end of stream.
            thread::sleep(Duration::from_millis(50));
            let mut got = Vec::new();
            ours.read_to_end(&mut got).unwrap();
            got
        });
        core.close_clients();
        assert_eq!(reader.join().unwrap(), bytes);
    }

    #[test]
    fn a_connection_that_stays_silent_is_dropped() {
        let mut core = core();
        let _socket = TcpStream::connect(core.local_addr().unwrap()).unwrap();
        let mut events = Vec::new();
        core.accept(&mut |e| events.push(e)).unwrap();
        assert_eq!(core.clients.len(), 1);
        core.read_clients(&mut |e| events.push(e));
        assert_eq!(core.clients.len(), 1, "still within the handshake window");
        core.clients[0].connected = Instant::now()
            .checked_sub(HANDSHAKE_TIMEOUT + Duration::from_secs(1))
            .unwrap();
        core.read_clients(&mut |e| events.push(e));
        assert!(core.clients.is_empty());
        assert!(events.iter().any(
            |e| matches!(e, Event::ClientDropped { reason, .. } if reason.contains("no connection settings"))
        ));
    }
}
