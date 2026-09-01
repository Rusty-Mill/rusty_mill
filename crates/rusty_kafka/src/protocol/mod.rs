//! Kafka protocol message types: request/response structs plus their
//! `encode`/`decode` methods, one module per API. Every API implemented
//! here uses its **v0** wire format -- the oldest, non-flexible/
//! non-compact encoding -- documented per-module rather than negotiated
//! via [`api_versions`] yet; see the crate's module doc for that
//! limitation.

pub mod api_versions;
pub mod create_topics;
pub mod header;
pub mod metadata;

/// Kafka API key constants for the requests this crate implements. The
/// full registry is much larger (see the [Kafka protocol
/// guide](https://kafka.apache.org/protocol.html#protocol_api_keys));
/// only the ones this crate actually sends are listed.
pub mod api_key {
    /// `Produce` -- not yet implemented by this crate (see the module
    /// doc); listed here since [`crate::protocol::api_versions`]
    /// decodes a broker's advertised version range for every API key it
    /// supports, including this one.
    pub const PRODUCE: i16 = 0;
    /// `Metadata`, implemented by [`crate::protocol::metadata`].
    pub const METADATA: i16 = 3;
    /// `ApiVersions`, implemented by [`crate::protocol::api_versions`].
    pub const API_VERSIONS: i16 = 18;
    /// `CreateTopics`, implemented by [`crate::protocol::create_topics`].
    pub const CREATE_TOPICS: i16 = 19;
}
