//! A stand-in for RLBot core: a listener that speaks the same frames, built from the wire
//! crate's own encoders.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use rb_rlbot_wire::frame::{frame, FrameDecoder};
use rb_rlbot_wire::{
    ControllableInfo, ControllableTeamInfo, CoreMessage, FieldInfo, GamePacket, InterfaceMessage,
    MatchConfiguration, MatchInfo, MatchPhase,
};

/// A core that has not been connected to yet.
pub struct FakeCore {
    listener: TcpListener,
}

impl FakeCore {
    pub fn start() -> FakeCore {
        FakeCore {
            listener: TcpListener::bind("127.0.0.1:0").unwrap(),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.listener.local_addr().unwrap()
    }

    pub fn accept(&self) -> Peer {
        let (stream, _) = self.listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        Peer {
            stream,
            decoder: FrameDecoder::new(),
        }
    }
}

/// Core's end of one connection.
pub struct Peer {
    stream: TcpStream,
    decoder: FrameDecoder,
}

impl Peer {
    pub fn send(&mut self, message: &CoreMessage) {
        let bytes = frame(&message.to_payload().unwrap()).unwrap();
        self.stream.write_all(&bytes).unwrap();
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).unwrap();
        self.stream.flush().unwrap();
    }

    /// The next message from the client; panics after ten seconds of silence.
    pub fn recv(&mut self) -> InterfaceMessage {
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(payload) = self.decoder.next_frame() {
                return InterfaceMessage::from_payload(&payload).unwrap();
            }
            let n = self.stream.read(&mut chunk).unwrap();
            assert!(n > 0, "client closed the connection");
            self.decoder.push(&chunk[..n]);
        }
    }

    /// True once the client has closed its end.
    pub fn is_closed(&mut self) -> bool {
        let mut byte = [0u8; 1];
        matches!(self.stream.read(&mut byte), Ok(0))
    }

    /// Core's usual opening: team, match and field information.
    pub fn send_starting_info(&mut self, team: u32, cars: &[u32]) {
        self.send(&CoreMessage::ControllableTeamInfo(team_info(team, cars)));
        self.send(&CoreMessage::MatchConfiguration(
            MatchConfiguration::default(),
        ));
        self.send(&CoreMessage::FieldInfo(FieldInfo::default()));
    }
}

pub fn team_info(team: u32, cars: &[u32]) -> ControllableTeamInfo {
    ControllableTeamInfo {
        team,
        controllables: cars
            .iter()
            .map(|&index| ControllableInfo {
                index,
                identifier: i32::try_from(index).unwrap() + 100,
            })
            .collect(),
    }
}

pub fn packet(frame_num: u32) -> CoreMessage {
    CoreMessage::GamePacket(GamePacket {
        players: Vec::new(),
        boost_pads: Vec::new(),
        balls: Vec::new(),
        match_info: MatchInfo {
            seconds_elapsed: 0.0,
            game_time_remaining: 300.0,
            is_overtime: false,
            is_unlimited_time: false,
            match_phase: MatchPhase::Active,
            frame_num,
        },
        teams: Vec::new(),
    })
}
