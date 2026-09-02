//! `FindCoordinator` (API key 10) v0: locates the broker acting as
//! coordinator for a consumer group -- the broker
//! `JoinGroup`/`SyncGroup`/`Heartbeat`/`LeaveGroup`/`OffsetCommit`
//! must all be sent to.
//!
//! **Scope caveat, not a wire-format concern**, matching
//! [`crate::protocol::offset_fetch`]'s own module doc:
//! [`crate::KafkaClient`] has no controller/coordinator discovery --
//! every request still goes out over the one connection it was
//! constructed with (see the crate's own module doc). This module
//! decodes a real `FindCoordinatorResponse` (`node_id`/`host`/`port`),
//! but nothing in this crate acts on it by opening a second
//! connection; correct only when the connected broker is also the
//! coordinator, true for meshed's single all-in-one dev broker, not
//! guaranteed in general.

use crate::error::CodecError;
use crate::wire::{read_i16, read_i32, read_string, write_i16, write_i32, write_string};
use rusty_wire::{Reader, Writer};

/// `FindCoordinatorRequest` v0.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FindCoordinatorRequest {
    /// The consumer group ID to find a coordinator for.
    pub group_id: String,
}

impl FindCoordinatorRequest {
    /// Encodes the v0 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_string(writer, &self.group_id);
    }

    /// Decodes a v0 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see
    /// [`crate::testing`]; this crate is client-only).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        Ok(FindCoordinatorRequest {
            group_id: read_string(reader)?,
        })
    }
}

/// `FindCoordinatorResponse` v0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindCoordinatorResponse {
    /// Kafka error code; `0` means success (e.g. `15` =
    /// `COORDINATOR_NOT_AVAILABLE`, `16` = `NOT_COORDINATOR`).
    pub error_code: i16,
    /// The coordinator broker's node ID.
    pub node_id: i32,
    /// The coordinator broker's host.
    pub host: String,
    /// The coordinator broker's port.
    pub port: i32,
}

impl FindCoordinatorResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        Ok(FindCoordinatorResponse {
            error_code: read_i16(reader)?,
            node_id: read_i32(reader)?,
            host: read_string(reader)?,
            port: read_i32(reader)?,
        })
    }

    /// Encodes the response body -- symmetric with
    /// [`decode`](Self::decode), for a fake broker standing in for
    /// tests.
    pub fn encode(&self, writer: &mut Writer) {
        write_i16(writer, self.error_code);
        write_i32(writer, self.node_id);
        write_string(writer, &self.host);
        write_i32(writer, self.port);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_encode_then_decode_round_trips() {
        let request = FindCoordinatorRequest {
            group_id: "readiness-reporting-personnel-consumer".to_string(),
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(
            FindCoordinatorRequest::decode(&mut reader).unwrap(),
            request
        );
    }

    #[test]
    fn response_decodes_a_successful_lookup() {
        let response = FindCoordinatorResponse {
            error_code: 0,
            node_id: 1,
            host: "localhost".to_string(),
            port: 9092,
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = FindCoordinatorResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn response_decodes_a_broker_error() {
        let response = FindCoordinatorResponse {
            error_code: 15, // COORDINATOR_NOT_AVAILABLE
            node_id: -1,
            host: String::new(),
            port: -1,
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = FindCoordinatorResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded.error_code, 15);
    }
}
