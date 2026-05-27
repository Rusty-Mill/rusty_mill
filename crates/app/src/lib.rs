//! `app` — the `Session` centre and its adapters (CLI today; gateway/Tauri
//! later). Implements `SessionFactory` in later phases.

mod embedder;
mod session;

pub use embedder::AiSdkEmbedder;
pub use session::{Session, TurnOutcome};
