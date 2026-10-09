use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use rb_rlbot_wire::frame::{frame, FrameDecoder};
use rb_rlbot_wire::{
    ConnectionSettings, ControllableTeamInfo, CoreMessage, FieldInfo, InterfaceMessage,
    MatchConfiguration, Ping,
};

use crate::{Error, Result};

/// The three messages core sends after a valid [`ConnectionSettings`].
#[derive(Debug, Clone, PartialEq)]
pub struct StartingInfo {
    pub controllable_team_info: ControllableTeamInfo,
    pub match_configuration: MatchConfiguration,
    pub field_info: FieldInfo,
}

/// A framed, blocking connection to RLBot core.
///
/// Bytes are buffered across reads, so a timeout in the middle of a frame loses nothing.
#[derive(Debug)]
pub struct Connection {
    stream: TcpStream,
    decoder: FrameDecoder,
}

impl Connection {
    /// Connects to core, usually `127.0.0.1:23234`.
    pub fn connect(addr: impl ToSocketAddrs) -> Result<Connection> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        Ok(Connection {
            stream,
            decoder: FrameDecoder::new(),
        })
    }

    /// Sends one message.
    pub fn send(&mut self, message: impl Into<InterfaceMessage>) -> Result<()> {
        self.send_all(&[message.into()])
    }

    /// Sends the messages in one write, in order.
    pub fn send_all(&mut self, messages: &[InterfaceMessage]) -> Result<()> {
        let mut bytes = Vec::new();
        for message in messages {
            bytes.extend_from_slice(&frame(&message.to_payload()?)?);
        }
        self.stream.write_all(&bytes)?;
        Ok(())
    }

    /// Waits for the next message from core.
    pub fn recv(&mut self) -> Result<CoreMessage> {
        self.stream.set_read_timeout(None)?;
        match read_message(&mut self.stream, &mut self.decoder, None)? {
            Some(message) => Ok(message),
            None => Err(Error::Io(io::ErrorKind::TimedOut.into())),
        }
    }

    /// Waits up to `timeout` for the next message; `None` if none arrived in time. Bytes of a
    /// half-received message are kept for the next call. A zero timeout is a poll: a message
    /// already buffered or readable right now is returned, and nothing is waited for.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<CoreMessage>> {
        read_message(
            &mut self.stream,
            &mut self.decoder,
            Some(Instant::now() + timeout),
        )
    }

    /// Introduces this client: sends `settings`, then waits for core's team, match and field
    /// information, answering pings meanwhile. Other messages are dropped. Send
    /// [`rb_rlbot_wire::InitComplete`] once ready.
    ///
    /// For agents (bots, hiveminds, scripts), which must have a non-empty `agent_id`: core
    /// sends the team information only to a named agent, so for an empty id the wait would never
    /// end. An id of nothing but whitespace counts as empty, because core trims it first. That is [`Error::EmptyAgentId`], returned before anything is sent. A match runner
    /// with no id sends its settings with [`Connection::send`] and reads what it wants with
    /// [`Connection::recv`], as `rb_match_log` does.
    pub fn handshake(&mut self, settings: ConnectionSettings) -> Result<StartingInfo> {
        // Core trims the id before deciding whether the agent is anonymous, so do the same.
        if settings.agent_id.trim().is_empty() {
            return Err(Error::EmptyAgentId);
        }
        self.send(settings)?;
        let (mut team, mut config, mut field) = (None, None, None);
        loop {
            match self.recv()? {
                CoreMessage::ControllableTeamInfo(m) => team = Some(m),
                CoreMessage::MatchConfiguration(m) => config = Some(m),
                CoreMessage::FieldInfo(m) => field = Some(m),
                CoreMessage::PingRequest(Ping { cookie }) => {
                    self.send(InterfaceMessage::PingResponse(Ping { cookie }))?;
                }
                CoreMessage::DisconnectSignal => return Err(Error::Disconnected),
                CoreMessage::GamePacket(_) | CoreMessage::Other(_) => {}
            }
            if let (Some(t), Some(c), Some(f)) = (&team, &config, &field) {
                return Ok(StartingInfo {
                    controllable_team_info: t.clone(),
                    match_configuration: c.clone(),
                    field_info: f.clone(),
                });
            }
        }
    }
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// How long a read may wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    /// Until data arrives.
    Forever,
    /// At most this long (never zero).
    UpTo(Duration),
    /// Not at all: only what is already there.
    Poll,
}

/// What `read_message` needs from a connection: the time, and a read with a bound on the wait.
/// A trait so the deadline logic can be tested with scripted reads and a scripted clock.
trait Transport {
    fn now(&self) -> Instant;
    fn read_within(&mut self, buf: &mut [u8], wait: Wait) -> io::Result<usize>;
}

impl Transport for TcpStream {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn read_within(&mut self, buf: &mut [u8], wait: Wait) -> io::Result<usize> {
        match wait {
            Wait::Forever => {
                self.set_read_timeout(None)?;
                self.read(buf)
            }
            Wait::UpTo(limit) => {
                self.set_read_timeout(Some(limit))?;
                self.read(buf)
            }
            Wait::Poll => {
                self.set_nonblocking(true)?;
                let read = self.read(buf);
                self.set_nonblocking(false)?;
                read
            }
        }
    }
}

/// The next decoded message, reading until a whole frame is buffered.
///
/// With a deadline: `None` once it has passed. The deadline is checked on every pass, so bytes
/// that keep arriving in small pieces cannot hold the call past it; the pieces stay in
/// `decoder` for the next call. Once it has passed (a zero timeout included) only what is already
/// readable is taken, without waiting.
fn read_message<T: Transport>(
    transport: &mut T,
    decoder: &mut FrameDecoder,
    deadline: Option<Instant>,
) -> Result<Option<CoreMessage>> {
    let mut chunk = [0u8; 8192];
    loop {
        if let Some(payload) = decoder.next_frame() {
            return Ok(Some(CoreMessage::from_payload(&payload)?));
        }
        let wait = match deadline {
            None => Wait::Forever,
            Some(deadline) => {
                let left = deadline.saturating_duration_since(transport.now());
                if left.is_zero() {
                    Wait::Poll
                } else {
                    Wait::UpTo(left)
                }
            }
        };
        match transport.read_within(&mut chunk, wait) {
            Ok(0) => return Err(Error::Closed),
            Ok(n) => decoder.push(&chunk[..n]),
            Err(e) if is_timeout(&e) && wait == Wait::Poll => return Ok(None),
            Err(e) if is_timeout(&e) || e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Scripted reads and clock: each read hands out `per_read` bytes of `bytes` and moves the
    /// clock on by `step`; a `Poll` finds nothing. Records every wait it was asked for.
    struct Scripted {
        base: Instant,
        elapsed: Duration,
        step: Duration,
        bytes: Vec<u8>,
        per_read: usize,
        waits: Vec<Wait>,
    }

    impl Scripted {
        fn new(bytes: Vec<u8>, per_read: usize, step: Duration) -> Scripted {
            Scripted {
                base: Instant::now(),
                elapsed: Duration::ZERO,
                step,
                bytes,
                per_read,
                waits: Vec::new(),
            }
        }
    }

    impl Transport for Scripted {
        fn now(&self) -> Instant {
            self.base + self.elapsed
        }

        fn read_within(&mut self, buf: &mut [u8], wait: Wait) -> io::Result<usize> {
            self.waits.push(wait);
            if wait == Wait::Poll || self.bytes.is_empty() {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            self.elapsed += self.step;
            let n = self.per_read.min(self.bytes.len()).min(buf.len());
            buf[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes.drain(..n);
            Ok(n)
        }
    }

    /// The start of a frame that claims 60,000 bytes and never finishes.
    fn endless_frame() -> Vec<u8> {
        let mut bytes = 60_000u16.to_be_bytes().to_vec();
        bytes.resize(60_002, 7);
        bytes
    }

    #[test]
    fn reads_that_keep_succeeding_cannot_run_past_the_deadline() {
        // A byte per read, 10 ms per read, 100 ms allowed. Every read succeeds, so no timeout
        // error ever occurs to prompt a deadline check: only the check on each pass stops it.
        let mut transport = Scripted::new(endless_frame(), 1, Duration::from_millis(10));
        let mut decoder = FrameDecoder::new();
        let deadline = transport.now() + Duration::from_millis(100);

        let got = read_message(&mut transport, &mut decoder, Some(deadline)).unwrap();

        assert_eq!(got, None);
        let timed_reads = transport
            .waits
            .iter()
            .filter(|w| matches!(w, Wait::UpTo(_)))
            .count();
        assert_eq!(timed_reads, 10, "waits: {:?}", transport.waits);
        assert_eq!(
            transport.waits.last(),
            Some(&Wait::Poll),
            "ends with a poll"
        );
        assert!(
            transport.bytes.len() > 59_000,
            "most of the stream was never read"
        );
    }

    #[test]
    fn a_wait_never_exceeds_what_is_left_and_is_never_zero() {
        let mut transport = Scripted::new(endless_frame(), 1, Duration::from_millis(30));
        let mut decoder = FrameDecoder::new();
        let deadline = transport.now() + Duration::from_millis(100);
        read_message(&mut transport, &mut decoder, Some(deadline)).unwrap();
        let left = |elapsed_ms: u64| Duration::from_millis(100 - elapsed_ms);
        assert_eq!(
            transport.waits,
            vec![
                Wait::UpTo(left(0)),
                Wait::UpTo(left(30)),
                Wait::UpTo(left(60)),
                Wait::UpTo(left(90)),
                Wait::Poll,
            ]
        );
    }

    #[test]
    fn a_zero_timeout_only_ever_polls() {
        let mut transport = Scripted::new(endless_frame(), 1, Duration::from_millis(10));
        let mut decoder = FrameDecoder::new();
        let deadline = transport.now();
        let got = read_message(&mut transport, &mut decoder, Some(deadline)).unwrap();
        assert_eq!(got, None);
        assert_eq!(
            transport.waits,
            vec![Wait::Poll],
            "one poll, no waiting read"
        );
    }

    #[test]
    fn a_poll_that_finds_a_whole_message_returns_it_and_a_buffered_one_needs_no_read() {
        let message = CoreMessage::DisconnectSignal;
        let framed = frame(&message.to_payload().unwrap()).unwrap();
        let mut decoder = FrameDecoder::new();
        // Already in the decoder: no read at all.
        decoder.push(&framed);
        let mut transport = Scripted::new(Vec::new(), 1, Duration::ZERO);
        let deadline = transport.now();
        assert_eq!(
            read_message(&mut transport, &mut decoder, Some(deadline)).unwrap(),
            Some(message.clone())
        );
        assert!(transport.waits.is_empty());

        // Data that is there when polled: read, and returned, without a timed wait.
        struct Ready(Vec<u8>);
        impl Transport for Ready {
            fn now(&self) -> Instant {
                Instant::now()
            }
            fn read_within(&mut self, buf: &mut [u8], wait: Wait) -> io::Result<usize> {
                assert_eq!(wait, Wait::Poll, "an expired deadline must not wait");
                if self.0.is_empty() {
                    return Err(io::ErrorKind::WouldBlock.into());
                }
                let n = self.0.len().min(buf.len());
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0.clear();
                Ok(n)
            }
        }
        let mut transport = Ready(framed);
        let mut decoder = FrameDecoder::new();
        let deadline = Instant::now();
        assert_eq!(
            read_message(&mut transport, &mut decoder, Some(deadline)).unwrap(),
            Some(message)
        );
    }

    #[test]
    fn without_a_deadline_every_read_waits_for_ever_until_the_frame_is_whole() {
        let message = CoreMessage::DisconnectSignal;
        let framed = frame(&message.to_payload().unwrap()).unwrap();
        let reads = framed.len().div_ceil(5);
        let mut transport = Scripted::new(framed, 5, Duration::from_secs(3600));
        let mut decoder = FrameDecoder::new();

        let got = read_message(&mut transport, &mut decoder, None).unwrap();

        assert_eq!(got, Some(message));
        assert_eq!(transport.waits, vec![Wait::Forever; reads]);
    }
}
