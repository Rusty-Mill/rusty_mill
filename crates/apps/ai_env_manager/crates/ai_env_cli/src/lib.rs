//! `aienv`: the Ai Env Manager command line (pack D4, D11). Thin on purpose:
//! parsing is pure ([`args`]), planning is the core's, load/plan/apply is
//! `ai_env_app`'s (over `ai_env_fs`). `enable`, `disable` and `edit` show their plan
//! and apply it only with `--yes` (D6).

pub mod args;
mod edit;
mod run;

pub use edit::Session;
pub use run::{run, Error, Outcome};
