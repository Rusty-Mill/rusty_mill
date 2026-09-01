//! Kafka protocol message types: request/response structs plus their
//! `encode`/`decode` methods, one module per API. Most APIs implemented
//! here use their **v0** wire format -- the oldest, non-flexible/
//! non-compact encoding -- documented per-module rather than negotiated
//! via [`api_versions`] yet; see the crate's module doc for that
//! limitation. [`list_offsets`] (v1) and [`produce`] (v3) are the
//! deliberate exceptions -- see each module's own doc for why; both
//! stay within classic (non-flexible) encoding regardless.

pub mod api_versions;
pub mod create_topics;
pub mod header;
pub mod list_offsets;
pub mod metadata;
pub mod offset_fetch;
pub mod produce;

/// Kafka API key constants for the requests this crate implements. The
/// full registry is much larger (see the [Kafka protocol
/// guide](https://kafka.apache.org/protocol.html#protocol_api_keys));
/// only the ones this crate actually sends are listed.
pub mod api_key {
    /// `Produce`, implemented (at v3) by [`crate::protocol::produce`].
    pub const PRODUCE: i16 = 0;
    /// `ListOffsets`, implemented (at v1) by
    /// [`crate::protocol::list_offsets`].
    pub const LIST_OFFSETS: i16 = 2;
    /// `Metadata`, implemented by [`crate::protocol::metadata`].
    pub const METADATA: i16 = 3;
    /// `OffsetFetch`, implemented by [`crate::protocol::offset_fetch`].
    pub const OFFSET_FETCH: i16 = 9;
    /// `ApiVersions`, implemented by [`crate::protocol::api_versions`].
    pub const API_VERSIONS: i16 = 18;
    /// `CreateTopics`, implemented by [`crate::protocol::create_topics`].
    pub const CREATE_TOPICS: i16 = 19;
}
