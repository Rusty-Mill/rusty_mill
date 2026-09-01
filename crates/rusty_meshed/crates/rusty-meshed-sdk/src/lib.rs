//! The data-product SDK -- the Rust port of `meshed.sdk` and
//! `meshed.infrastructure` (`BaseEvent`, `DataProductProducerBase`/
//! `DataProductConsumerBase`, `RegistryClient`, the transactional
//! outbox, and topic naming/lifecycle management via `TopicManager`).
//!
//! See `../../capability-manifest.md` (rows SDK-001..086) for the
//! capabilities this crate covers; most are still open, see the crate's
//! module list below for what's implemented so far.

mod topic_config;
mod topic_manager;

pub use topic_config::{TopicSpec, TopicType};
pub use topic_manager::{
    validate_topic_name, CreateTopicError, TopicManager, TopicNameError, TopicStatus,
    WELL_KNOWN_STREAM_TYPES,
};
