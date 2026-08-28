//! `rusty_url` is a from-scratch implementation of the [WHATWG URL Living
//! Standard](https://url.spec.whatwg.org/), built to reach public-API parity
//! with the [`url`](https://crates.io/crates/url) crate. See `gap-analysis.md`
//! in the repository root for the tracked parity roadmap.

mod error;
mod file_path;
mod host;
mod origin;
mod parser;
mod path_segments;
mod percent_encode;
mod position;
mod url;

pub mod form_urlencoded;

pub use error::ParseError;
pub use host::Host;
pub use origin::{OpaqueOrigin, Origin};
pub use parser::SyntaxViolation;
pub use path_segments::PathSegmentsMut;
pub use position::Position;
pub use url::{ParseOptions, Url, UrlQuery};
