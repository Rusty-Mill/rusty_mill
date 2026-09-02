//! `Heartbeat` (API key 12) v0: keeps a consumer group membership
//! alive between polls, telling the coordinator this member is still
//! active without triggering a rebalance.

use crate::error::CodecError;
use crate::wire::{read_i16, read_i32, read_string, write_i16, write_i32, write_string};
use rusty_wire::{Reader, Writer};

/// `HeartbeatRequest` v0.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeartbeatRequest {
    pub group_id: String,
    /// The generation this member last synced with (from
    /// `JoinGroupResponse`/`SyncGroupResponse`).
    pub generation_id: i32,
    /// This member's ID, assigned by the coordinator in
    /// `JoinGroupResponse`.
    pub member_id: String,
}

impl HeartbeatRequest {
    /// Encodes the v0 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_string(writer, &self.group_id);
        write_i32(writer, self.generation_id);
        write_string(writer, &self.member_id);
    }

    /// Decodes a v0 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see
    /// [`crate::testing`]; this crate is client-only).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        Ok(HeartbeatRequest {
            group_id: read_string(reader)?,
            generation_id: read_i32(reader)?,
            member_id: read_string(reader)?,
        })
    }
}

/// `HeartbeatResponse` v0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatResponse {
    /// Kafka error code; `0` means success (e.g. `27` =
    /// `REBALANCE_IN_PROGRESS` -- the caller must rejoin via
    /// `JoinGroup`, `25` = `UNKNOWN_MEMBER_ID`).
    pub error_code: i16,
}

impl HeartbeatResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        Ok(HeartbeatResponse {
            error_code: read_i16(reader)?,
        })
    }

    /// Encodes the response body -- symmetric with
    /// [`decode`](Self::decode), for a fake broker standing in for
    /// tests.
    pub fn encode(&self, writer: &mut Writer) {
        write_i16(writer, self.error_code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_encode_then_decode_round_trips() {
        let request = HeartbeatRequest {
            group_id: "readiness-reporting-personnel-consumer".to_string(),
            generation_id: 3,
            member_id: "consumer-1-abc".to_string(),
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(HeartbeatRequest::decode(&mut reader).unwrap(), request);
    }

    #[test]
    fn response_decodes_success() {
        let response = HeartbeatResponse { error_code: 0 };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(HeartbeatResponse::decode(&mut reader).unwrap(), response);
    }

    #[test]
    fn response_decodes_rebalance_in_progress() {
        let response = HeartbeatResponse { error_code: 27 };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(
            HeartbeatResponse::decode(&mut reader).unwrap().error_code,
            27
        );
    }
}
