//! The manpower domain data products -- the Rust port of
//! `meshed.domains` (personnel/position/readiness event schemas, the
//! three domain producers, the readiness-reporting consumers,
//! `ScenarioBuilder`, and the `run_continuous`/`run_scenario` demo
//! generators).
//!
//! See `../../capability-manifest.md` (rows DOM-001..047) for the
//! capabilities this crate covers. The event schemas (DOM-001..011,
//! [`events`]), the three domain products/two derivation consumers
//! (DOM-012..025, [`products`]), [`generators::ScenarioBuilder`]
//! (DOM-026..036), and the `run_continuous`/`run_scenario` demo
//! binaries (`src/bin/`, DOM-037..047) are all implemented.

pub mod events;
pub mod generators;
pub mod products;
