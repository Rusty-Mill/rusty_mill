//! 3D **replay viewer**: turn a canonical match into a self-contained, playable
//! HTML scene.
//!
//! A pure consumer of [`replay_analyzer`]'s [`CanonicalMatch`] (like `scoring`,
//! `skills`, and `value`): [`scene::build_scene`] distills the resampled grid,
//! events, and detected skills into a compact [`scene::Scene`], and
//! [`render::html`] embeds it into a three.js viewer. No parsing, no I/O in the
//! core — deterministic and golden-testable; the only non-determinism is the
//! browser that renders the output.
//!
//! # Example
//! ```no_run
//! use replay_analyzer::{analyze::build_canonical, decode::{ReplayParser, boxcars_adapter::BoxcarsParser}};
//! use replay_skills::{detect_all, SkillConfig};
//! use replay_viewer::{build_scene, html};
//! let data = std::fs::read("match.replay").unwrap();
//! let decoded = BoxcarsParser::new().parse(&data).unwrap();
//! let canonical = build_canonical(&decoded, "match");
//! let skills = detect_all(&canonical, &SkillConfig::default());
//! let scene = build_scene(&canonical, &skills.instances);
//! std::fs::write("match.html", html(&scene)).unwrap();
//! ```

pub mod render;
pub mod scene;

pub use render::html;
pub use scene::{build_scene, Scene};
