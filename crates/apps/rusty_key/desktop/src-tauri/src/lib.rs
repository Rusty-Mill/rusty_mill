//! Rusty Keys desktop shell (Tauri 2 / Phase 15).
//!
//! The frontend is a reactive rendering layer over `rk_app::Session`; this crate
//! is the Tauri bridge between them. It registers one command per
//! `rk_app::contract::command` name, emits the canonical `rk://` events
//! (`rk_app::contract::event`), and renders turn failures via the boundary error
//! taxonomy — all referencing the single contract SSOT so the event/command
//! surface cannot drift from the gateway and ACP adapters.

pub mod bridge;
pub mod error;
pub mod secrets;
pub mod state;

use tauri::{Builder, Manager, Runtime};

use state::{AppState, ApprovalRx};

/// The Tauri `invoke` command names this bridge registers — the contract SSOT.
pub const COMMANDS: [&str; 23] = rk_app::contract::command::ALL;
/// The `rk://` events this bridge emits — the contract SSOT.
pub const EVENTS: [&str; 9] = rk_app::contract::event::ALL;

/// Wire an [`AppState`] and the approval receiver onto a Tauri builder: manage the
/// state, spawn the task that turns gate requests into `rk://approval_request`
/// events, and register every contract command. Generic over the runtime so the
/// IPC smoke test configures the mock runtime identically to production.
pub fn configure<R: Runtime>(builder: Builder<R>, state: AppState, rx: ApprovalRx) -> Builder<R> {
    builder
        .manage(state)
        .setup(move |app| {
            let handle = app.handle().clone();
            let mut rx = rx;
            tauri::async_runtime::spawn(async move {
                while let Some(req) = rx.recv().await {
                    let state = handle.state::<AppState>();
                    bridge::emit_approval_request(&handle, state.inner(), req);
                }
            });
            Ok(())
        })
        .invoke_handler(bridge::invoke_handler())
}

/// Build and run the production desktop app (the real `OpenAICompatible` session).
pub fn run() -> anyhow::Result<()> {
    let (state, rx) = state::build_production_state()?;
    bridge::assert_catalogs();
    configure(Builder::default(), state, rx)
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("tauri runtime error: {e}"))?;
    Ok(())
}
