// Suppress the extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = rusty_keys_desktop_lib::run() {
        eprintln!("rusty-keys desktop failed to start: {e}");
        std::process::exit(1);
    }
}
