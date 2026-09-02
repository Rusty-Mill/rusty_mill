//! `LeaveGroup` (API key 13) v0: voluntarily leaves a consumer group,
//! so the coordinator can trigger a rebalance immediately instead of
//! waiting out this member's session timeout.

use crate::error::CodecError;
use crate::wire::{read_i16, read_string, write_i16, write_string};
use rusty_wire::{Reader, Writer};

/// `LeaveGroupRequest` v0.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LeaveGroupRequest {
    pub group_id: String,
    /// This member's ID, assigned by the coordinator in
    /// `JoinGroupResponse`.
    pub member_id: String,
}

impl LeaveGroupRequest {
    /// Encodes the v0 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_string(writer, &self.group_id);
        write_string(writer, &self.member_id);
    }

    /// Decodes a v0 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see
    /// [`crate::testing`]; this crate is client-only).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        Ok(LeaveGroupRequest {
            group_id: read_string(reader)?,
            member_id: read_string(reader)?,
        })
    }
}

/// `LeaveGroupResponse` v0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaveGroupResponse {
    /// Kafka error code; `0` means success.
    pub error_code: i16,
}

impl LeaveGroupResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        Ok(LeaveGroupResponse {
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
        let request = LeaveGroupRequest {
            group_id: "readiness-reporting-personnel-consumer".to_string(),
            member_id: "consumer-1-abc".to_string(),
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(LeaveGroupRequest::decode(&mut reader).unwrap(), request);
    }

    #[test]
    fn response_decodes_success() {
        let response = LeaveGroupResponse { error_code: 0 };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(LeaveGroupResponse::decode(&mut reader).unwrap(), response);
    }
}
