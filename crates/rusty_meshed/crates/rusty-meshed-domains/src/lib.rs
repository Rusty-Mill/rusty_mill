//! The manpower domain data products -- the Rust port of
//! `meshed.domains` (personnel/position/readiness event schemas, the
//! three domain producers, the readiness-reporting consumers,
//! `ScenarioBuilder`, and the `run_continuous`/`run_scenario` demo
//! generators).
//!
//! See `../../capability-manifest.md` (rows DOM-001..047) for the
//! capabilities this crate covers. The event schemas (DOM-001..011,
//! [`events`]) and the three domain products/two derivation consumers
//! (DOM-012..025, [`products`] -- partially: see that module's own doc
//! for what's still blocked on `rusty_kafka`'s `Fetch` gap) are
//! implemented; `ScenarioBuilder` and the `run_continuous`/
//! `run_scenario` demo generators (DOM-026..047) are not yet.

pub mod events;
pub mod products;
