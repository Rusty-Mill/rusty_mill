//! `rusty_erosion`: a small code-erosion metric for the Let Em Loose erosion
//! gate (card #704, D41–D45, D52).
//!
//! Two shares, in basis points of the measured code:
//! - **CC concentration:** cyclomatic complexity in functions above CC 10,
//!   over all CC.
//! - **Duplication:** tokens inside a repeated window of 50 normalized
//!   tokens, over all tokens.
//!
//! The gate compares a candidate with its pinned baseline ([`Report::delta`]),
//! never the absolute level. v0 measures Rust only; other code is listed as
//! `unmeasured`. The metric is a heuristic and can be gamed by splitting
//! functions or renaming; the delta's counts exist so calibration can watch.
//!
//! ```
//! let base = rusty_erosion::measure([("src/a.rs", "fn a() {}")])?;
//! let cand = rusty_erosion::measure([("src/a.rs", "fn a() { if x {} }")])?;
//! assert_eq!(base.delta(&cand).total_cc, 1);
//! # Ok::<(), rusty_erosion::Error>(())
//! ```

pub mod adapter;
pub mod core;

pub use adapter::{Error, FileClass, classify, measure};
pub use core::{Affected, Delta, FileMeasure, FunctionMetric, Lang, Ratio, Report};
