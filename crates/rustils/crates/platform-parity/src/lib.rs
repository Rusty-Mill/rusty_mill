//! # platform-parity — the shared behavior-spec assertion sets
//!
//! One copy of every cross-backend parity assertion, so a backend's
//! parity suite is a list of which sets apply to it rather than a
//! transcription of them.
//!
//! ## Why this crate exists
//!
//! `net_parity.rs` and `security_parity.rs` each carried their own
//! doc-comment note recording the intended trigger: *extract once a
//! third backend would otherwise mean a third copy.* `platform-bsd`
//! (rustils#48/#86) made net the third, and `TrustAnchors`
//! (rustils#88) made security the third. This is that extraction.
//!
//! The trigger was the right one, and the copies had already started to
//! rot by the time it fired — two ways, both found while extracting:
//!
//! - `platform-bsd`'s `assert_net_behavior` had lost two explanatory
//!   comments the other two still carried. Harmless, but it is exactly
//!   how three copies stop being one spec.
//! - `assert_credential_store_behavior` had genuinely **diverged**:
//!   Windows scoped its test service name by process id, Linux used a
//!   fixed string. Against a real per-user OS credential store, two
//!   concurrently-running test binaries sharing one fixed name can see
//!   each other's writes. The pid-scoped version is the correct one and
//!   is what this crate carries — so extracting fixed a latent flake
//!   rather than merely deduplicating text. (The same class of bug the
//!   net suite already fixed once, when `assert_unix_behavior`'s socket
//!   path gained a per-backend label for the same reason.)
//!
//! ## What belongs here, and what does not
//!
//! **Here:** assertions written against `platform`'s traits, true of
//! every backend that implements them.
//!
//! **Not here:** anything a single backend owns. Per-backend `#[test]`
//! wrappers stay in that backend's own suite, because *which* sets apply
//! is a per-backend fact — `platform-bsd` is net-only plus
//! `TrustAnchors`, and Unix-socket assertions mean nothing on Windows.
//! Backend-specific suites (`tun_parity.rs`, `net_nonblocking.rs`,
//! `secret_service.rs`) stay put entirely.
//!
//! This crate depends on `platform` and nothing else — deliberately, and
//! notably **not** on `platform-mock`. An assertion set that could see a
//! backend would be able to accommodate one.
//!
//! ## Not a shipped crate
//!
//! Test support only, consumed as a `dev-dependency`. It never reaches
//! a dependency graph any real consumer builds.

pub mod net;
pub mod security;
