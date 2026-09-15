//! # threading — minimal multithreading primitives
//!
//! Scoped-thread spawn with a decoded join outcome, and `Mutex`/`RwLock`
//! wrappers with an explicit poisoning policy (Atlas `ATLAS-STATE-0001`:
//! "Introducing shared mutable state MUST include an explicit
//! synchronization strategy").
//!
//! Deliberately minimal: `rusty_foundation_akb`'s own threading
//! capability doc (`docs/02-capabilities/threading/`) is still a *Draft
//! domain analysis* — it has settled conclusions about thread lifecycle
//! and mutex/rw-lock semantics, but wait primitives (condition
//! variables, semaphores), atomics policy, and scheduling/affinity are
//! still open. This crate builds only what that doc already treats as
//! settled; building the rest now would be exactly the speculative
//! abstraction Atlas's Economy value (`ATLAS-NONGOAL-0030`/`0031`)
//! exists to prevent. Extend this crate's scope when that doc's status
//! changes, not before.

#![forbid(unsafe_code)]

pub mod scope;
pub mod sync;

pub use scope::{scope, JoinOutcome, Scope, ScopedJoinHandle};
pub use sync::{Mutex, PoisonPolicy, RwLock};
