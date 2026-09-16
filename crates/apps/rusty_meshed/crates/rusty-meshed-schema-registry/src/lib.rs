//! Schema Registry client and compatibility-mode enforcement -- the Rust
//! port of `meshed.schema_registry` (`SchemaRegistryEnforcer`,
//! `CompatibilityMode`, `CompatibilityViolation`).
//!
//! See `../../capability-manifest.md` (rows REG-140..152) for the
//! capabilities this crate covers.

mod client;
mod models;

pub use client::{
    RegisterSchemaError, SchemaRegistryEnforcer, SchemaRegistryError, SetCompatibilityError,
};
pub use models::{CompatibilityMode, CompatibilityViolation};
