//! The data-product SDK -- the Rust port of `meshed.sdk` and
//! `meshed.infrastructure` (`BaseEvent`, `DataProductProducerBase`/
//! `DataProductConsumerBase`, `RegistryClient`, the transactional
//! outbox, and topic naming/lifecycle management via `TopicManager`).
//!
//! See `../../capability-manifest.md` (rows SDK-001..086) for the
//! capabilities this crate covers; most are still open, see the crate's
//! module list below for what's implemented so far.

mod consumer;
mod error;
pub mod outbox;
mod producer;
pub mod registry_client;
mod topic_config;
mod topic_manager;
mod types;

/// `rusty_meshed_sdk::BaseEvent` -- included in this crate's own
/// re-export surface for parity with `meshed.sdk.__all__` (SDK-080),
/// even though the type itself lives in `rusty-meshed-core` (see that
/// crate's own module doc for why: both this crate and
/// `rusty-meshed-domains` need it, and they're siblings with no
/// dependency edge between them).
pub use rusty_meshed_core::BaseEvent;

pub use consumer::{ConsumerStartupError, DataProductConsumerBase};
pub use error::{ContractVersionMismatch, RegistryError};
pub use outbox::{
    ensure_schema as ensure_outbox_schema, relay_pending, write_outbox_entry, OutboxEntry,
    OutboxRelay, RelayError,
};
pub use producer::{DataProductProducerBase, ProducerError, PublishError};
pub use topic_config::{TopicSpec, TopicType};
pub use topic_manager::{
    validate_topic_name, CreateTopicError, TopicManager, TopicNameError, TopicStatus,
    WELL_KNOWN_STREAM_TYPES,
};
pub use types::{OutputPortSpec, PortDescriptor};
