//! `init_registry` -- the Rust port of `scripts/init_registry.py`
//! (CLI-046): sets the Schema Registry's global compatibility to
//! `FULL_TRANSITIVE`. Run once after the local dev stack comes up so
//! every subject defaults to the platform's strictest compatibility
//! mode. The Schema Registry URL is read from
//! `MESHED_SCHEMA_REGISTRY_URL` (default `"http://localhost:8081"`).
//!
//! Thin wiring over an already-built capability:
//! `rusty-meshed-schema-registry::SchemaRegistryEnforcer::initialize_global_compatibility`
//! (REG-140-series) already does the actual work.

use rusty_meshed_schema_registry::SchemaRegistryEnforcer;

#[rusty_tokio::main]
async fn main() {
    let url = std::env::var("MESHED_SCHEMA_REGISTRY_URL")
        .unwrap_or_else(|_| "http://localhost:8081".to_string());

    let enforcer = SchemaRegistryEnforcer::new(url.clone());
    match enforcer.initialize_global_compatibility().await {
        Ok(()) => {
            println!("Schema Registry at {url}: global compatibility set to FULL_TRANSITIVE");
        }
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    }
}
