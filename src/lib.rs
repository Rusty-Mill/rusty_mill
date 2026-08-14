//! `rusty_oauth`: a hand-rolled, zero-dependency implementation of the
//! OAuth 2.0 / 2.1 protocol family for Rust.
//!
//! Every primitive this crate needs -- SHA-256, HMAC-SHA256, Base64,
//! percent-encoding, a JSON reader/writer, and a CSPRNG -- is implemented
//! from scratch in-tree (see [`crypto`], [`encoding`], [`json`], and
//! [`rand`]), so the crate has **no runtime dependencies**.
//!
//! # Scope: protocol logic, not transport
//!
//! This crate deliberately does **not** ship an HTTP client or a TLS
//! implementation. Hand-rolling TLS is a correctness- and security-critical
//! undertaking that a general-purpose OAuth crate has no business
//! attempting; instead, `rusty_oauth` builds fully-formed requests
//! (method, URL, headers, body) and parses responses you fetch with
//! whatever HTTP client you already trust (`reqwest`, `ureq`, `hyper`,
//! a raw `std::net::TcpStream` + your own TLS stack, etc). This mirrors
//! the design of most serious OAuth libraries (e.g. `oauth2-rs`).
//!
//! # Standards implemented
//!
//! - RFC 6749 -- The OAuth 2.0 Authorization Framework
//! - RFC 6750 -- Bearer Token Usage
//! - RFC 7009 -- Token Revocation
//! - RFC 7517 -- JSON Web Key (JWK), including JWK Set parsing and
//!   `kid`-based key selection
//! - RFC 7519 -- JSON Web Token (JWT)
//! - RFC 7515 -- JSON Web Signature (JWS), HS256 and RS256
//! - RFC 7523 -- JWT Profile for OAuth 2.0 Client Authentication and
//!   Authorization Grants (bearer assertions)
//! - RFC 7636 -- Proof Key for Code Exchange (PKCE)
//! - RFC 7662 -- Token Introspection
//! - RFC 8414 -- Authorization Server Metadata
//! - RFC 8628 -- Device Authorization Grant
//! - RFC 9126 -- Pushed Authorization Requests (PAR)
//! - OAuth 2.1 (draft-ietf-oauth-v2-1): PKCE is applied by default to every
//!   authorization-code flow, and the deprecated Implicit and Resource
//!   Owner Password Credentials grants are intentionally not implemented.
//!
//! # Example: authorization code flow with PKCE
//!
//! ```no_run
//! use rusty_oauth::client::{ClientId, Client};
//! use rusty_oauth::authorization::AuthorizationRequest;
//! use rusty_oauth::pkce::Pkce;
//!
//! let client = Client::public(ClientId::new("my-client-id"));
//! let pkce = Pkce::generate().unwrap();
//! let request = AuthorizationRequest::new(
//!     "https://auth.example.com/authorize",
//!     &client,
//!     "https://app.example.com/callback",
//! )
//! .scope("openid profile")
//! .pkce(&pkce)
//! .build()
//! .unwrap();
//!
//! // Send the user to `request.url`, remembering `request.state` and
//! // `pkce.code_verifier` (e.g. in a server-side session) until the
//! // callback arrives.
//! println!("{}", request.url);
//! ```

pub mod authorization;
pub mod bearer;
pub mod client;
pub mod crypto;
pub mod device;
pub mod encoding;
pub mod error;
pub mod introspection;
pub mod json;
pub mod jwks;
pub mod jwt;
pub mod metadata;
pub mod par;
pub mod pkce;
pub mod rand;
pub mod request;
pub mod revocation;
pub mod token;

pub use error::{Error, Result};
