use rb_rlbot_wire::{
    ConnectionSettings, ControllableInfo, CoreMessage, FieldInfo, GamePacket, InitComplete,
    InterfaceMessage, MatchConfiguration, Ping,
};

use crate::connection::StartingInfo;
use crate::{Connection, Result};

/// Messages an agent wants sent once its callback returns.
#[derive(Debug, Default)]
pub struct Outbox {
    queue: Vec<InterfaceMessage>,
}

impl Outbox {
    pub fn push(&mut self, message: impl Into<InterfaceMessage>) {
        self.queue.push(message.into());
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn flush(&mut self, connection: &mut Connection) -> Result<()> {
        if self.queue.is_empty() {
            return Ok(());
        }
        let queue = std::mem::take(&mut self.queue);
        connection.send_all(&queue)
    }
}

/// Something that reacts to game packets: a bot, a hivemind or a script.
pub trait Agent {
    /// Called once per [`GamePacket`]. Queue [`rb_rlbot_wire::PlayerInput`],
    /// [`rb_rlbot_wire::DesiredGameState`] and the like in `out`.
    fn tick(&mut self, packet: &GamePacket, out: &mut Outbox);
}

/// What a bot is told about the car it controls.
#[derive(Debug, Clone, Copy)]
pub struct BotInit<'a> {
    pub team: u32,
    pub controllable: ControllableInfo,
    pub match_configuration: &'a MatchConfiguration,
    pub field_info: &'a FieldInfo,
}

/// Connects as a bot: one agent per controlled car, all ticked on every packet.
///
/// `make` runs once per car after the handshake and may queue messages. Returns `Ok` when core
/// disconnects the client or controls no cars, and `Err` if the connection fails.
pub fn run_bots<A: Agent>(
    connection: &mut Connection,
    settings: ConnectionSettings,
    mut make: impl FnMut(BotInit<'_>, &mut Outbox) -> A,
) -> Result<()> {
    let info = connection.handshake(settings)?;
    let team = info.controllable_team_info.team;
    if info.controllable_team_info.controllables.is_empty() {
        return Ok(());
    }
    let mut out = Outbox::default();
    let agents: Vec<A> = info
        .controllable_team_info
        .controllables
        .iter()
        .map(|&controllable| {
            make(
                BotInit {
                    team,
                    controllable,
                    match_configuration: &info.match_configuration,
                    field_info: &info.field_info,
                },
                &mut out,
            )
        })
        .collect();
    serve(connection, agents, out)
}

/// Connects as a hivemind or script: one agent for everything this connection controls.
///
/// `make` runs once after the handshake and may queue messages. Returns `Ok` when core
/// disconnects the client, `Err` if the connection fails.
pub fn run_hivemind<A: Agent>(
    connection: &mut Connection,
    settings: ConnectionSettings,
    make: impl FnOnce(&StartingInfo, &mut Outbox) -> A,
) -> Result<()> {
    let info = connection.handshake(settings)?;
    let mut out = Outbox::default();
    let agent = make(&info, &mut out);
    serve(connection, vec![agent], out)
}

/// Announces readiness, then ticks every agent on each game packet until core disconnects us.
fn serve<A: Agent>(connection: &mut Connection, mut agents: Vec<A>, mut out: Outbox) -> Result<()> {
    out.push(InitComplete);
    out.flush(connection)?;
    loop {
        match connection.recv()? {
            CoreMessage::DisconnectSignal => return Ok(()),
            CoreMessage::GamePacket(packet) => {
                for agent in &mut agents {
                    agent.tick(&packet, &mut out);
                }
            }
            CoreMessage::PingRequest(Ping { cookie }) => {
                out.push(InterfaceMessage::PingResponse(Ping { cookie }));
            }
            CoreMessage::ControllableTeamInfo(_)
            | CoreMessage::MatchConfiguration(_)
            | CoreMessage::FieldInfo(_)
            | CoreMessage::Other(_) => {}
        }
        out.flush(connection)?;
    }
}
