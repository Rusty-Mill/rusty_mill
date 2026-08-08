//! Route matching and policy evaluation.
//!
//! A [`Router`] is built once from an [`agentgateway_config::Config`] and then
//! answers [`Router::select`] per request. Building is where regexes compile
//! and routes get sorted into precedence order, so the request path does no
//! work that could have been done at startup.

mod cors;
mod headers;
mod hostname;
mod ratelimit;
mod retry;
mod rewrite;
mod router;

pub use cors::{CorsDecision, CorsMatcher};
pub use headers::{HeaderError, Headers};
pub use hostname::HostnamePattern;
pub use ratelimit::{RateLimitError, RateLimiter, RetryAfter};
pub use retry::Retry;
pub use rewrite::{Rewrite, RewriteError, parse_authority};
pub use router::{
    CompiledBind, CompiledListener, CompiledRoute, RouteMatcher, Router, RouterError, Selection,
};

#[cfg(test)]
mod tests;
