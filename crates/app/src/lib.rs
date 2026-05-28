//! `app` — the `Session` centre and its adapters (CLI today; gateway/Tauri
//! later). Implements `SessionFactory` in later phases.

mod budget;
pub mod cli;
mod embedder;
pub mod eval;
#[cfg(feature = "gateway")]
pub mod gateway;
mod session;

pub use budget::{Msg, Tier, TokenBudget};
pub use cli::Stats;
pub use embedder::AiSdkEmbedder;
pub use eval::{run_episode, EvalOutcome, GoldenEpisode};
pub use session::{Session, TurnOutcome};
