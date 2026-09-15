//! # reactor-core — runtime-agnostic async-io primitives
//!
//! `rusty_foundation_akb` [ADR-0160](https://github.com/Rusty-Mill/rusty_foundation_akb/blob/main/docs/adr/0160-async-io-lifecycle-is-a-provider-framework-not-a-universal-capability.md)
//! decided that async I/O lifecycle is a *provider framework*, not a
//! universal capability: shared lifecycle and safety plumbing, with
//! domain semantics (what "progress" means for a file read vs. a
//! process wait) staying in the domain crate. This crate is that
//! plumbing and nothing else — see `platform-async` for the domain
//! surface built on top of it.
//!
//! Constraints this crate holds itself to, traced to specific AKB
//! requirements:
//! - **No hidden runtime** (`RM-ASYNC-RUNTIME-0001`): nothing here
//!   spawns a thread, owns a global executor, or assumes a particular
//!   async runtime is present. Callers bring their own
//!   [`std::task::Waker`] (delivered by whatever executor is polling
//!   them) and this crate's own [`Clock`].
//! - **Generation-scoped operation identity** (`RM-ASYNC-OP-0001`):
//!   every operation gets a provider-unique id scoped to an engine
//!   generation, so a completion from a torn-down engine can never be
//!   mistaken for one from its replacement.
//! - **Explicit cancellation** (`RM-ASYNC-CANCEL-0001`): cancellation is
//!   a caller-supplied token, not an implicit `Drop`-only mechanism.

#![forbid(unsafe_code)]

pub mod cancellation;
pub mod clock;
pub mod operation;
pub mod shutdown;

pub use cancellation::CancellationToken;
pub use clock::{Clock, SystemClock};
pub use operation::{Generation, OperationId, OperationIdIssuer};
pub use shutdown::ShutdownSignal;
