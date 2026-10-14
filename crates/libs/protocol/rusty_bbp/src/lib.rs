//! Blackboard Protocol core. Pure: no I/O, no clock, no randomness.
//!
//! `engine::handle` turns a `Command` into events and a response. `TaskState`
//! is the fold over events. `store` holds the storage trait and an in-memory
//! implementation for stage 1.

#![forbid(unsafe_code)]

pub mod codec;
pub mod command;
pub mod engine;
pub mod event;
pub mod fs_store;
pub mod ids;
pub mod metrics;
pub mod policy;
pub mod record;
pub mod role;
pub mod shadow;
pub mod state;
pub mod store;
pub mod wake;

pub use codec::{decode_artifact, encode_artifact, SpecFile};
pub use command::{AgentAction, Code, Command, HumanAction, OpenTask, Rejection, Response};
pub use engine::{handle, Handled};
pub use event::{
    log_schema, schema_for, schema_supported, EscalateReason, Event, TurnEnd, LOG_SCHEMA,
    LOG_SCHEMA_CONCURRENT,
};
pub use fs_store::{log_path_for, FsError, FsStore};
pub use ids::*;
pub use policy::SchedulingPolicy;
pub use record::*;
pub use role::*;
pub use shadow::{Comparison, Shadow, ShadowKind};
pub use state::{Budget, BudgetField, Card, Gate, Slice, State, TaskState, Turn, TurnKind};
pub use store::{Driver, MemStore, ShadowSink, Store, StoreError};
pub use wake::{can_act, wake_set, WakeReason};
