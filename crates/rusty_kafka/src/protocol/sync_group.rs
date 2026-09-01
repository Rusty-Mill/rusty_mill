//! `SyncGroup` (API key 14) v0: the second half of a rebalance --
//! every group member calls this after `JoinGroup`; the leader's call
//! carries the partition assignment for the whole group (computed
//! from every member's [`crate::protocol::join_group::JoinGroupMember`]
//! metadata), every other member sends an empty assignment list. The
//! coordinator distributes the leader's assignment back to everyone
//! via each member's own response.

use crate::error::CodecError;
use crate::wire::{
    read_array_len, read_i16, read_i32, read_nullable_bytes, read_string, write_i16, write_i32,
    write_nullable_bytes, write_string,
};
use rusty_wire::{Reader, Writer};

/// One member's assignment within a [`SyncGroupRequest`] -- only the
/// group leader's request populates this (every other member sends an
/// empty `Vec`); see [`crate::protocol::join_group::JoinGroupResponse::is_leader`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncGroupAssignment {
    pub member_id: String,
    /// That member's assignment payload -- a
    /// `ConsumerProtocolAssignment` (see
    /// [`crate::protocol::consumer_protocol::encode_assignment`]) when
    /// `protocol_type = "consumer"`, opaque bytes as far as
    /// `SyncGroup` itself is concerned.
    pub assignment: Vec<u8>,
}

/// `SyncGroupRequest` v0.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncGroupRequest {
    pub group_id: String,
    /// The generation from [`crate::protocol::join_group::JoinGroupResponse::generation_id`].
    pub generation_id: i32,
    /// This connection's own member ID, from
    /// [`crate::protocol::join_group::JoinGroupResponse::member_id`].
    pub member_id: String,
    /// The whole group's assignment, if this connection is the
    /// leader; empty otherwise.
    pub assignments: Vec<SyncGroupAssignment>,
}

impl SyncGroupRequest {
    /// Encodes the v0 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_string(writer, &self.group_id);
        write_i32(writer, self.generation_id);
        write_string(writer, &self.member_id);
        write_i32(writer, self.assignments.len() as i32);
        for assignment in &self.assignments {
            write_string(writer, &assignment.member_id);
            write_nullable_bytes(writer, Some(&assignment.assignment));
        }
    }

    /// Decodes a v0 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see
    /// [`crate::testing`]; this crate is client-only).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let group_id = read_string(reader)?;
        let generation_id = read_i32(reader)?;
        let member_id = read_string(reader)?;
        let assignment_count = read_array_len(reader)?.max(0);
        let mut assignments = Vec::with_capacity(assignment_count as usize);
        for _ in 0..assignment_count {
            let assignment_member_id = read_string(reader)?;
            let assignment = read_nullable_bytes(reader)?.unwrap_or(&[]).to_vec();
            assignments.push(SyncGroupAssignment {
                member_id: assignment_member_id,
                assignment,
            });
        }
        Ok(SyncGroupRequest {
            group_id,
            generation_id,
            member_id,
            assignments,
        })
    }
}

/// `SyncGroupResponse` v0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncGroupResponse {
    /// Kafka error code; `0` means success (e.g. `27` =
    /// `REBALANCE_IN_PROGRESS`, `22` = `ILLEGAL_GENERATION`).
    pub error_code: i16,
    /// This connection's own partition assignment -- a
    /// `ConsumerProtocolAssignment` payload (see
    /// [`crate::protocol::consumer_protocol::decode_assignment`]).
    pub assignment: Vec<u8>,
}

impl SyncGroupResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        Ok(SyncGroupResponse {
            error_code: read_i16(reader)?,
            assignment: read_nullable_bytes(reader)?.unwrap_or(&[]).to_vec(),
        })
    }

    /// Encodes the response body -- symmetric with
    /// [`decode`](Self::decode), for a fake broker standing in for
    /// tests.
    pub fn encode(&self, writer: &mut Writer) {
        write_i16(writer, self.error_code);
        write_nullable_bytes(writer, Some(&self.assignment));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_encode_then_decode_round_trips_a_leader_assignment() {
        let request = SyncGroupRequest {
            group_id: "readiness-reporting-personnel-consumer".to_string(),
            generation_id: 1,
            member_id: "consumer-1-abc".to_string(),
            assignments: vec![SyncGroupAssignment {
                member_id: "consumer-1-abc".to_string(),
                assignment: crate::protocol::consumer_protocol::encode_assignment(&[(
                    "manpower.personnel-lifecycle.assignments".to_string(),
                    vec![0, 1, 2],
                )]),
            }],
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(SyncGroupRequest::decode(&mut reader).unwrap(), request);
    }

    #[test]
    fn request_encode_then_decode_round_trips_an_empty_follower_assignment() {
        let request = SyncGroupRequest {
            group_id: "g".to_string(),
            generation_id: 1,
            member_id: "consumer-2-def".to_string(),
            assignments: vec![],
        };
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = SyncGroupRequest::decode(&mut reader).unwrap();
        assert!(decoded.assignments.is_empty());
    }

    #[test]
    fn response_decodes_the_assigned_partitions() {
        let assignment = crate::protocol::consumer_protocol::encode_assignment(&[(
            "manpower.personnel-lifecycle.assignments".to_string(),
            vec![0, 1, 2],
        )]);
        let response = SyncGroupResponse {
            error_code: 0,
            assignment: assignment.clone(),
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = SyncGroupResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded, response);
        assert_eq!(
            crate::protocol::consumer_protocol::decode_assignment(&decoded.assignment).unwrap(),
            vec![(
                "manpower.personnel-lifecycle.assignments".to_string(),
                vec![0, 1, 2]
            )]
        );
    }
}
