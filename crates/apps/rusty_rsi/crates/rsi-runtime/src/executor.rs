//! [`ProcessExecutor`], the sandboxed [`rsi_core::Executor`] (ADR-0005
//! §4, invariant 5). It lives in `rusty_sandbox` since ADR-0007 follow-ons
//! step 7; this module keeps the path. Its errors are `rusty_sandbox`'s
//! and convert into [`crate::RuntimeError`].

pub use rusty_sandbox::executor::*;
