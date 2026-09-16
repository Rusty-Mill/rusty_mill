//! The clipboard scratchpad's collector.
//!
//! Off by default and re-checked on every tick, so turning it off in the UI
//! stops collection immediately rather than at the next restart. Nothing is
//! stored while it is off.

use crate::AppState;
use std::time::Duration;
use tauri::Manager;
use tauri_plugin_clipboard_manager::ClipboardExt;

const POLL: Duration = Duration::from_millis(900);

pub fn spawn_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut last = String::new();
        loop {
            std::thread::sleep(POLL);

            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            // Consult the setting every tick — this is the opt-in.
            let enabled = state
                .inventory
                .lock()
                .ok()
                .and_then(|inv| inv.scratchpad_enabled().ok())
                .unwrap_or(false);
            if !enabled {
                last.clear();
                continue;
            }

            let Ok(text) = app.clipboard().read_text() else {
                continue;
            };
            if text.trim().is_empty() || text == last {
                continue;
            }
            last = text.clone();

            let app_name = frontmost_app();
            if let Ok(inv) = state.inventory.lock() {
                let _ = inv.remember_clip(&text, app_name.as_deref());
            };
        }
    });
}

/// Which app the clipboard entry came from.
///
/// macOS can answer this; the other platforms have no equivalent that works
/// without extra permissions, so the clip is stored untagged rather than
/// guessed at.
#[cfg(target_os = "macos")]
fn frontmost_app() -> Option<String> {
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to get name of first application process whose frontmost is true",
        ])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

#[cfg(not(target_os = "macos"))]
fn frontmost_app() -> Option<String> {
    None
}
