//! The Rust port of the source repo's own `main.py` (CLI-047):
//! vestigial scaffolding, 84 bytes in the source, not registered as a
//! `pyproject.toml` script and not referenced anywhere else in that
//! codebase. Preserved here as its own binary target (`cargo run --bin
//! main`) for the same reason -- present, unreferenced by the real
//! `meshed` binary or any other crate, exactly as unused as its source
//! counterpart.

fn main() {
    println!("Hello from meshed!");
}
