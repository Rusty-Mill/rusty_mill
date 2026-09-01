//! The manpower domain data products -- the Rust port of
//! `meshed.domains` (personnel/position/readiness event schemas, the
//! three domain producers, the readiness-reporting consumers,
//! `ScenarioBuilder`, and the `run_continuous`/`run_scenario` demo
//! generators).
//!
//! See `../../capability-manifest.md` (rows DOM-001..047) for the
//! capabilities this crate covers; the event schemas (DOM-001..011,
//! [`events`]) are implemented, the producers/consumers/scenario
//! builder/demo generators (DOM-012..047) are not yet.

pub mod events;
