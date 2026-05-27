//! `app` — the `Session` centre and its adapters (CLI today; gateway/Tauri
//! later). Implements `SessionFactory` in later phases.

mod session;

pub use session::{Session, TurnOutcome};
