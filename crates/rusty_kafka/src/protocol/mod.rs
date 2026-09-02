//! Kafka protocol message types: request/response structs plus their
//! `encode`/`decode` methods, one module per API. Most APIs implemented
//! here use their **v0** wire format -- the oldest, non-flexible/
//! non-compact encoding -- documented per-module rather than negotiated
//! via [`api_versions`] yet; see the crate's module doc for that
//! limitation. [`list_offsets`] (v1), [`produce`] (v3), and [`fetch`]
//! (v4) are the deliberate exceptions -- see each module's own doc for
//! why; all stay within classic (non-flexible) encoding regardless.
//!
//! [`join_group`]/[`sync_group`] carry an embedded protocol payload
//! (the `metadata`/`assignment` `BYTES` fields) with its own encoding,
//! independent of whichever version of `JoinGroup`/`SyncGroup` itself
//! carries it -- see [`consumer_protocol`]'s own module doc.

pub mod api_versions;
pub mod consumer_protocol;
pub mod create_topics;
pub mod fetch;
pub mod find_coordinator;
pub mod header;
pub mod heartbeat;
pub mod join_group;
pub mod leave_group;
pub mod list_offsets;
pub mod metadata;
pub mod offset_commit;
pub mod offset_fetch;
pub mod produce;
pub mod sync_group;

/// Kafka API key constants for the requests this crate implements. The
/// full registry is much larger (see the [Kafka protocol
/// guide](https://kafka.apache.org/protocol.html#protocol_api_keys));
/// only the ones this crate actually sends are listed.
pub mod api_key {
    /// `Produce`, implemented (at v3) by [`crate::protocol::produce`].
    pub const PRODUCE: i16 = 0;
    /// `Fetch`, implemented (at v4) by [`crate::protocol::fetch`].
    pub const FETCH: i16 = 1;
    /// `ListOffsets`, implemented (at v1) by
    /// [`crate::protocol::list_offsets`].
    pub const LIST_OFFSETS: i16 = 2;
    /// `Metadata`, implemented by [`crate::protocol::metadata`].
    pub const METADATA: i16 = 3;
    /// `OffsetCommit`, implemented (at v2) by
    /// [`crate::protocol::offset_commit`].
    pub const OFFSET_COMMIT: i16 = 8;
    /// `OffsetFetch`, implemented by [`crate::protocol::offset_fetch`].
    pub const OFFSET_FETCH: i16 = 9;
    /// `FindCoordinator`, implemented by
    /// [`crate::protocol::find_coordinator`].
    pub const FIND_COORDINATOR: i16 = 10;
    /// `JoinGroup`, implemented by [`crate::protocol::join_group`].
    pub const JOIN_GROUP: i16 = 11;
    /// `Heartbeat`, implemented by [`crate::protocol::heartbeat`].
    pub const HEARTBEAT: i16 = 12;
    /// `LeaveGroup`, implemented by [`crate::protocol::leave_group`].
    pub const LEAVE_GROUP: i16 = 13;
    /// `SyncGroup`, implemented by [`crate::protocol::sync_group`].
    pub const SYNC_GROUP: i16 = 14;
    /// `ApiVersions`, implemented by [`crate::protocol::api_versions`].
    pub const API_VERSIONS: i16 = 18;
    /// `CreateTopics`, implemented by [`crate::protocol::create_topics`].
    pub const CREATE_TOPICS: i16 = 19;
}
