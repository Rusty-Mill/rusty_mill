//! `rusty_text`: a real (subset, honestly-documented) sed engine and awk
//! engine, both built on `rusty_regx`. See [`sed`] and [`awk`]'s own module
//! docs for exactly what's implemented vs. deliberately out of scope.

pub mod awk;
pub mod sed;

pub use awk::AwkProgram;
pub use sed::SedScript;
