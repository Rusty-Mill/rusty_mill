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
        match self.read_message(None)? {
            Some(message) => Ok(message),
            None => Err(Error::Io(io::ErrorKind::TimedOut.into())),
        }
    }

    /// Waits up to `timeout` for the next message; `None` if none arrived in time.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<CoreMessage>> {
        self.read_message(Some(Instant::now() + timeout))
    }

    /// Introduces this client: sends `settings`, then waits for core's team, match and field
    /// information, answering pings meanwhile. Other messages are dropped. Send
    /// [`rb_rlbot_wire::InitComplete`] once ready.
    pub fn handshake(&mut self, settings: ConnectionSettings) -> Result<StartingInfo> {
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

    /// The next decoded message, reading the socket until a whole frame is buffered.
    /// With a deadline, `None` once it passes.
    fn read_message(&mut self, deadline: Option<Instant>) -> Result<Option<CoreMessage>> {
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(payload) = self.decoder.next_frame() {
                return Ok(Some(CoreMessage::from_payload(&payload)?));
            }
            if let Some(deadline) = deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                // A zero timeout means "block forever" to the OS, so ask for at least 1 ms.
                self.stream
                    .set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(Error::Closed),
                Ok(n) => self.decoder.push(&chunk[..n]),
                Err(e) if is_timeout(&e) => {
                    if deadline.is_some_and(|d| Instant::now() >= d) {
                        return Ok(None);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
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
