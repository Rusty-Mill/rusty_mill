// Tauri's own convention: a thin shim so mobile targets, which need a
// `cdylib` entry point rather than a `main.rs`, can link the same
// library crate (`lib.rs`'s own `run()`) without this file at all.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    sessionmgr_desktop_lib::run();
}
