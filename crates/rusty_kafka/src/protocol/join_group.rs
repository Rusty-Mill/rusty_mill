//! `JoinGroup` (API key 11) v0: joins (or creates) a consumer group.
//! The coordinator picks one member as the group's leader -- the only
//! member whose response carries the full `members` list -- and it's
//! the leader's job to decide partition assignment and hand it back
//! via `SyncGroup`.
//!
//! This crate implements eager rebalancing only: every join sends the
//! full topic subscription and, on the very first join, an empty
//! `member_id` (the coordinator assigns a fresh one and returns it via
//! [`JoinGroupResponse::member_id`], expected back on every subsequent
//! `JoinGroup`/`SyncGroup`/`Heartbeat`/`LeaveGroup`/`OffsetCommit` call
//! for this membership). No cooperative-incremental rebalancing
//! (KIP-429) -- see [`crate::protocol::consumer_protocol`]'s own
//! module doc for the wire-format consequence.

use crate::error::CodecError;
use crate::wire::{
    read_array_len, read_i16, read_i32, read_nullable_bytes, read_string, write_i16, write_i32,
    write_nullable_bytes, write_string,
};
use rusty_wire::{Reader, Writer};

/// One protocol this member is willing to use within a
/// [`JoinGroupRequest`] -- `protocol_name` is a well-known assignment
/// strategy name (`"range"`, `"roundrobin"`, ...); `metadata` is that
/// protocol's own payload, opaque to `JoinGroup` itself (see
/// [`crate::protocol::consumer_protocol`] for the `protocol_type =
/// "consumer"` payload this crate actually sends).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGroupProtocol {
    pub name: String,
    pub metadata: Vec<u8>,
}

/// `JoinGroupRequest` v0.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JoinGroupRequest {
    pub group_id: String,
    /// How long the coordinator waits without a `Heartbeat` before
    /// considering this member dead and triggering a rebalance.
    pub session_timeout_ms: i32,
    /// Empty on a first join; the ID the coordinator assigned via a
    /// previous [`JoinGroupResponse`] on every subsequent one.
    pub member_id: String,
    /// The embedded-protocol family this group speaks -- always
    /// `"consumer"` for this crate's callers.
    pub protocol_type: String,
    pub protocols: Vec<JoinGroupProtocol>,
}

impl JoinGroupRequest {
    /// Encodes the v0 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_string(writer, &self.group_id);
        write_i32(writer, self.session_timeout_ms);
        write_string(writer, &self.member_id);
        write_string(writer, &self.protocol_type);
        write_i32(writer, self.protocols.len() as i32);
        for protocol in &self.protocols {
            write_string(writer, &protocol.name);
            write_nullable_bytes(writer, Some(&protocol.metadata));
        }
    }

    /// Decodes a v0 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see
    /// [`crate::testing`]; this crate is client-only).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let group_id = read_string(reader)?;
        let session_timeout_ms = read_i32(reader)?;
        let member_id = read_string(reader)?;
        let protocol_type = read_string(reader)?;
        let protocol_count = read_array_len(reader)?.max(0);
        let mut protocols = Vec::with_capacity(protocol_count as usize);
        for _ in 0..protocol_count {
            let name = read_string(reader)?;
            let metadata = read_nullable_bytes(reader)?.unwrap_or(&[]).to_vec();
            protocols.push(JoinGroupProtocol { name, metadata });
        }
        Ok(JoinGroupRequest {
            group_id,
            session_timeout_ms,
            member_id,
            protocol_type,
            protocols,
        })
    }
}

/// One other group member within a [`JoinGroupResponse`] -- only
/// populated (by every version of the real protocol) in the response
/// sent to whichever member the coordinator elected leader; every
/// other member's response carries an empty `members` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGroupMember {
    pub member_id: String,
    /// That member's own protocol metadata (e.g. its
    /// `ConsumerProtocolSubscription` bytes) for whichever protocol
    /// name the group settled on
    /// ([`JoinGroupResponse::group_protocol`]).
    pub metadata: Vec<u8>,
}

/// `JoinGroupResponse` v0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGroupResponse {
    /// Kafka error code; `0` means success (e.g. `25` =
    /// `UNKNOWN_MEMBER_ID` -- rejoin with an empty `member_id`).
    pub error_code: i16,
    /// The group generation this membership belongs to -- echoed back
    /// on every subsequent `SyncGroup`/`Heartbeat`/`OffsetCommit`
    /// call; bumped by the coordinator on every rebalance.
    pub generation_id: i32,
    /// The protocol name the coordinator selected (the one every
    /// member's [`JoinGroupProtocol`] list had in common with the
    /// highest priority).
    pub group_protocol: String,
    /// The member ID of the elected group leader.
    pub leader_id: String,
    /// This connection's own member ID -- assigned fresh on a first
    /// join, echoed back on rejoin.
    pub member_id: String,
    /// Every group member's protocol metadata, non-empty only in the
    /// response sent to the leader (see [`JoinGroupMember`]'s own
    /// doc) -- the leader's job to turn into partition assignments,
    /// then send back via `SyncGroup`.
    pub members: Vec<JoinGroupMember>,
}

impl JoinGroupResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let error_code = read_i16(reader)?;
        let generation_id = read_i32(reader)?;
        let group_protocol = read_string(reader)?;
        let leader_id = read_string(reader)?;
        let member_id = read_string(reader)?;
        let member_count = read_array_len(reader)?.max(0);
        let mut members = Vec::with_capacity(member_count as usize);
        for _ in 0..member_count {
            let member_id = read_string(reader)?;
            let metadata = read_nullable_bytes(reader)?.unwrap_or(&[]).to_vec();
            members.push(JoinGroupMember {
                member_id,
                metadata,
            });
        }
        Ok(JoinGroupResponse {
            error_code,
            generation_id,
            group_protocol,
            leader_id,
            member_id,
            members,
        })
    }

    /// Encodes the response body -- symmetric with
    /// [`decode`](Self::decode), for a fake broker standing in for
    /// tests.
    pub fn encode(&self, writer: &mut Writer) {
        write_i16(writer, self.error_code);
        write_i32(writer, self.generation_id);
        write_string(writer, &self.group_protocol);
        write_string(writer, &self.leader_id);
        write_string(writer, &self.member_id);
        write_i32(writer, self.members.len() as i32);
        for member in &self.members {
            write_string(writer, &member.member_id);
            write_nullable_bytes(writer, Some(&member.metadata));
        }
    }

    /// Whether this connection's own member ID was elected group
    /// leader -- whoever is must compute partition assignments and
    /// send them via `SyncGroup`; everyone else sends an empty
    /// assignment list and waits for the leader's.
    pub fn is_leader(&self) -> bool {
        self.member_id == self.leader_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> JoinGroupRequest {
        JoinGroupRequest {
            group_id: "readiness-reporting-personnel-consumer".to_string(),
            session_timeout_ms: 10_000,
            member_id: String::new(),
            protocol_type: "consumer".to_string(),
            protocols: vec![JoinGroupProtocol {
                name: "range".to_string(),
                metadata: crate::protocol::consumer_protocol::encode_subscription(&[
                    "manpower.personnel-lifecycle.assignments".to_string(),
                ]),
            }],
        }
    }

    #[test]
    fn request_encode_then_decode_round_trips() {
        let request = sample_request();
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(JoinGroupRequest::decode(&mut reader).unwrap(), request);
    }

    #[test]
    fn request_sends_an_empty_member_id_on_first_join() {
        let request = sample_request();
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = JoinGroupRequest::decode(&mut reader).unwrap();
        assert_eq!(decoded.member_id, "");
    }

    #[test]
    fn response_decodes_when_this_member_is_the_leader() {
        let metadata = crate::protocol::consumer_protocol::encode_subscription(&["t".to_string()]);
        let response = JoinGroupResponse {
            error_code: 0,
            generation_id: 1,
            group_protocol: "range".to_string(),
            leader_id: "consumer-1-abc".to_string(),
            member_id: "consumer-1-abc".to_string(),
            members: vec![JoinGroupMember {
                member_id: "consumer-1-abc".to_string(),
                metadata: metadata.clone(),
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = JoinGroupResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded, response);
        assert!(decoded.is_leader());
        assert_eq!(
            crate::protocol::consumer_protocol::decode_subscription(&decoded.members[0].metadata)
                .unwrap(),
            vec!["t".to_string()]
        );
    }

    #[test]
    fn response_decodes_an_empty_members_list_for_a_follower() {
        let response = JoinGroupResponse {
            error_code: 0,
            generation_id: 1,
            group_protocol: "range".to_string(),
            leader_id: "consumer-1-abc".to_string(),
            member_id: "consumer-2-def".to_string(),
            members: vec![],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = JoinGroupResponse::decode(&mut reader).unwrap();
        assert!(decoded.members.is_empty());
        assert!(!decoded.is_leader());
    }
}
