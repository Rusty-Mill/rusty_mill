//! `app` — the `Session` centre and its adapters (CLI today; gateway/Tauri
//! later). Implements `SessionFactory` in later phases.

mod budget;
mod embedder;
pub mod eval;
mod session;

pub use budget::{Msg, Tier, TokenBudget};
pub use embedder::AiSdkEmbedder;
pub use eval::{run_episode, EvalOutcome, GoldenEpisode};
pub use session::{Session, TurnOutcome};
