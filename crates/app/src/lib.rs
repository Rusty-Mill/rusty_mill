//! `app` — the `Session` centre and its adapters (CLI today; gateway/Tauri
//! later). Implements `SessionFactory` in later phases.

mod budget;
mod embedder;
mod session;

pub use budget::{Msg, Tier, TokenBudget};
pub use embedder::AiSdkEmbedder;
pub use session::{Session, TurnOutcome};
