//! `sessionmgr`'s desktop front end (issue #23): a graphical client for
//! the same daemon `sessionmgr-tui` and the CLI already talk to, over
//! the same `AF_UNIX` socket protocol. Depends on `sessionmgr-protocol`
//! only -- see `paths.rs`'s module docs for why that boundary matters
//! here just as much as it does for the TUI.

mod attach;
mod client;
mod commands;
mod daemon;
mod paths;
mod unix_stream;

use std::collections::HashMap;
use std::sync::Mutex;

use commands::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            let root =
                paths::state_root().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            daemon::ensure_daemon(&root).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let socket = paths::daemon_socket(&root);
            _app.manage(AppState {
                socket,
                attaches: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::session_list,
            commands::session_new,
            commands::session_close,
            commands::session_rename,
            commands::session_fork,
            commands::session_switch_agent,
            commands::git_status,
            commands::git_diff,
            commands::attach_session,
            commands::detach_session,
            commands::send_input,
            commands::send_resize,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the sessionmgr desktop app");
}
