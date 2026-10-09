//! The two message unions. Every payload is a one-table packet: slot 0 holds the union tag,
//! slot 1 the message table (tag 0 is "none", so members count from 1).

use rusty_flatbuffers::{Builder, Offset, Table};

use crate::codec::{TableCodec, R};
use crate::tables::{ConnectionSettings, InitComplete, Ping, PlayerInput, StopCommand};
use crate::types::{
    ControllableTeamInfo, DesiredGameState, FieldInfo, GamePacket, MatchConfiguration,
};
use crate::Error;

/// Wraps an already-written message table in the packet table and finishes the buffer.
fn packet(tag: u8, message: impl FnOnce(&mut Builder) -> R<Offset>) -> R<Vec<u8>> {
    let mut b = Builder::new();
    let message = message(&mut b)?;
    let mut t = b.start_table();
    t.add_scalar(0, tag, 0)?;
    t.add_offset(1, message)?;
    let root = t.finish()?;
    Ok(b.finish(root, None)?)
}

/// The tag and message table of a payload.
fn open(payload: &[u8]) -> R<(u8, Table<'_>)> {
    let root = Table::root(payload)?;
    let tag = root.scalar(0, 0u8)?;
    let message = root.table(1)?.ok_or(Error::Missing("message"))?;
    Ok((tag, message))
}

/// What a client (bot, script or match runner) sends to core.
#[derive(Debug, Clone, PartialEq)]
pub enum InterfaceMessage {
    MatchConfiguration(MatchConfiguration),
    PlayerInput(PlayerInput),
    DesiredGameState(DesiredGameState),
    ConnectionSettings(ConnectionSettings),
    StopCommand(StopCommand),
    InitComplete(InitComplete),
    /// The answer to [`CoreMessage::PingRequest`].
    PingResponse(Ping),
}

const I_MATCH_CONFIGURATION: u8 = 3;
const I_PLAYER_INPUT: u8 = 4;
const I_DESIRED_GAME_STATE: u8 = 5;
const I_CONNECTION_SETTINGS: u8 = 9;
const I_STOP_COMMAND: u8 = 10;
const I_INIT_COMPLETE: u8 = 12;
const I_PING_RESPONSE: u8 = 15;

impl InterfaceMessage {
    /// The payload bytes, without the length prefix (see [`crate::frame`]).
    pub fn to_payload(&self) -> R<Vec<u8>> {
        match self {
            InterfaceMessage::MatchConfiguration(m) => {
                packet(I_MATCH_CONFIGURATION, |b| m.write(b))
            }
            InterfaceMessage::PlayerInput(m) => packet(I_PLAYER_INPUT, |b| m.write(b)),
            InterfaceMessage::DesiredGameState(m) => packet(I_DESIRED_GAME_STATE, |b| m.write(b)),
            InterfaceMessage::ConnectionSettings(m) => {
                packet(I_CONNECTION_SETTINGS, |b| m.write(b))
            }
            InterfaceMessage::StopCommand(m) => packet(I_STOP_COMMAND, |b| m.write(b)),
            InterfaceMessage::InitComplete(m) => packet(I_INIT_COMPLETE, |b| m.write(b)),
            InterfaceMessage::PingResponse(m) => packet(I_PING_RESPONSE, |b| m.write(b)),
        }
    }

    /// Decodes a payload (what a stand-in core receives). Members this crate does not model
    /// are [`Error::UnknownUnion`].
    pub fn from_payload(payload: &[u8]) -> R<InterfaceMessage> {
        let (tag, t) = open(payload)?;
        Ok(match tag {
            I_MATCH_CONFIGURATION => {
                InterfaceMessage::MatchConfiguration(MatchConfiguration::read(&t)?)
            }
            I_PLAYER_INPUT => InterfaceMessage::PlayerInput(PlayerInput::read(&t)?),
            I_DESIRED_GAME_STATE => InterfaceMessage::DesiredGameState(DesiredGameState::read(&t)?),
            I_CONNECTION_SETTINGS => {
                InterfaceMessage::ConnectionSettings(ConnectionSettings::read(&t)?)
            }
            I_STOP_COMMAND => InterfaceMessage::StopCommand(StopCommand::read(&t)?),
            I_INIT_COMPLETE => InterfaceMessage::InitComplete(InitComplete::read(&t)?),
            I_PING_RESPONSE => InterfaceMessage::PingResponse(Ping::read(&t)?),
            tag => {
                return Err(Error::UnknownUnion {
                    name: "InterfaceMessage",
                    tag,
                })
            }
        })
    }
}

/// What core sends to a client.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreMessage {
    GamePacket(GamePacket),
    FieldInfo(FieldInfo),
    MatchConfiguration(MatchConfiguration),
    ControllableTeamInfo(ControllableTeamInfo),
    /// The client should exit.
    DisconnectSignal,
    /// Core asks for a [`InterfaceMessage::PingResponse`] with the same cookie.
    PingRequest(Ping),
    /// A message type this crate does not model (comms, ball prediction, ...), by tag.
    Other(u8),
}

const C_DISCONNECT_SIGNAL: u8 = 1;
const C_GAME_PACKET: u8 = 2;
const C_FIELD_INFO: u8 = 3;
const C_MATCH_CONFIGURATION: u8 = 4;
const C_CONTROLLABLE_TEAM_INFO: u8 = 7;
const C_PING_REQUEST: u8 = 9;

impl CoreMessage {
    /// Decodes a payload. Unmodelled members are [`CoreMessage::Other`], not errors, because
    /// core sends many kinds a client may ignore.
    pub fn from_payload(payload: &[u8]) -> R<CoreMessage> {
        let root = Table::root(payload)?;
        let tag = root.scalar(0, 0u8)?;
        if tag == C_DISCONNECT_SIGNAL {
            return Ok(CoreMessage::DisconnectSignal);
        }
        if !matches!(
            tag,
            C_GAME_PACKET
                | C_FIELD_INFO
                | C_MATCH_CONFIGURATION
                | C_CONTROLLABLE_TEAM_INFO
                | C_PING_REQUEST
        ) {
            return Ok(CoreMessage::Other(tag));
        }
        let (_, t) = open(payload)?;
        Ok(match tag {
            C_GAME_PACKET => CoreMessage::GamePacket(GamePacket::read(&t)?),
            C_FIELD_INFO => CoreMessage::FieldInfo(FieldInfo::read(&t)?),
            C_MATCH_CONFIGURATION => CoreMessage::MatchConfiguration(MatchConfiguration::read(&t)?),
            C_PING_REQUEST => CoreMessage::PingRequest(Ping::read(&t)?),
            _ => CoreMessage::ControllableTeamInfo(ControllableTeamInfo::read(&t)?),
        })
    }

    /// The payload bytes (what a stand-in core sends). [`CoreMessage::Other`] cannot be built.
    pub fn to_payload(&self) -> R<Vec<u8>> {
        match self {
            CoreMessage::GamePacket(m) => packet(C_GAME_PACKET, |b| m.write(b)),
            CoreMessage::FieldInfo(m) => packet(C_FIELD_INFO, |b| m.write(b)),
            CoreMessage::MatchConfiguration(m) => packet(C_MATCH_CONFIGURATION, |b| m.write(b)),
            CoreMessage::ControllableTeamInfo(m) => {
                packet(C_CONTROLLABLE_TEAM_INFO, |b| m.write(b))
            }
            // `DisconnectSignal` is an empty table, the same shape as `InitComplete`.
            CoreMessage::DisconnectSignal => packet(C_DISCONNECT_SIGNAL, |b| InitComplete.write(b)),
            CoreMessage::PingRequest(m) => packet(C_PING_REQUEST, |b| m.write(b)),
            CoreMessage::Other(tag) => Err(Error::UnknownUnion {
                name: "CoreMessage",
                tag: *tag,
            }),
        }
    }
}

macro_rules! into_interface {
    ($($variant:ident($ty:ty)),+ $(,)?) => {$(
        impl From<$ty> for InterfaceMessage {
            fn from(m: $ty) -> Self {
                InterfaceMessage::$variant(m)
            }
        }
    )+};
}
into_interface!(
    MatchConfiguration(MatchConfiguration),
    PlayerInput(PlayerInput),
    DesiredGameState(DesiredGameState),
    ConnectionSettings(ConnectionSettings),
    StopCommand(StopCommand),
    InitComplete(InitComplete),
);
