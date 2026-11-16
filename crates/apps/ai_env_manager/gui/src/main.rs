// Suppress the extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = ai_env_gui_lib::run(std::env::args_os().skip(1)) {
        eprintln!("ai-env-gui: {e}");
        std::process::exit(1);
    }
}
