//! `aienv`: the Ai Env Manager command line (pack D4, D11). Thin on purpose:
//! parsing is pure ([`args`]), planning is the core's, disk access is
//! `ai_env_fs`'s. `enable` and `disable` show their plan and apply it only
//! with `--yes` (D6).

pub mod args;
mod run;

pub use run::{run, Error, Outcome};
