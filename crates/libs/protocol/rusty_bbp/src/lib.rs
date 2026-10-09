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
pub mod record;
pub mod state;
pub mod store;

pub use codec::{decode_artifact, encode_artifact, SpecFile};
pub use command::{AgentAction, Code, Command, HumanAction, OpenTask, Rejection, Response};
pub use engine::{handle, Handled};
pub use event::Event;
pub use fs_store::{FsError, FsStore};
pub use ids::*;
pub use record::*;
pub use state::{Budget, BudgetField, Card, Gate, State, TaskState, TurnKind};
pub use store::{Driver, MemStore, Store, StoreError};
