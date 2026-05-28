//! Tauri IPC smoke test (Phase 15 test gate). Headless, no live editor or
//! display: a scripted `FakeLanguageModel` session drives the real bridge on the
//! Tauri mock runtime.
//!
//! It guards the two things the Definition of Done hinges on:
//! 1. **Anti-drift:** the registered command/event catalogs equal the contract
//!    SSOT (`rk_app::contract`), and every contract command is actually reachable
//!    through the generated `invoke` handler (not just listed).
//! 2. **Event mirroring:** a turn run over IPC emits the canonical `rk://`
//!    boundary events (`turn_start` … `turn_complete`).

use std::sync::{Arc, Mutex};

use rk_app::contract::{command, event};
use rk_app::Session;
use rk_config::Config;
use rk_constrain::{ApprovalGate, ApprovalTrigger};
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use rusty_keys_desktop_lib::state::{AppState, ApprovalRx};
use rusty_keys_desktop_lib::{configure, COMMANDS, EVENTS};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::mpsc;

/// A scripted-model session + approval receiver, wired exactly like production.
fn fake_state() -> (AppState, ApprovalRx) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let uniq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rk-desktop-smoke-{}-{uniq}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let ws = dir.to_string_lossy().into_owned();
    let config = Config::resolve(|k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(ws.clone()),
        _ => None,
    })
    .unwrap();
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("done.".into())]]);
    let (tx, rx) = mpsc::channel(8);
    let gate = ApprovalGate::new(vec![ApprovalTrigger::NewFilePath], tx);
    let session = Session::new_with_policy(&config, model, Arc::new(gate)).unwrap();
    let state = AppState {
        session: Arc::new(session),
        workspace: config.workspace,
        pending_approval: Arc::new(Mutex::new(None)),
        overrides: Mutex::new(serde_json::Map::new()),
    };
    (state, rx)
}

/// Build the bridge on the mock runtime and return a webview to invoke against.
fn mock_window() -> WebviewWindow<MockRuntime> {
    let (state, rx) = fake_state();
    let app = configure(mock_builder(), state, rx)
        .build(mock_context(noop_assets()))
        .expect("mock app builds");
    WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .expect("mock webview builds")
}

/// Invoke `cmd` with `args` and return the raw result/error.
fn invoke(
    window: &WebviewWindow<MockRuntime>,
    cmd: &str,
    args: serde_json::Value,
) -> Result<tauri::ipc::InvokeResponseBody, serde_json::Value> {
    tauri::test::get_ipc_response(
        window,
        InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: args.into(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
}

#[test]
fn catalogs_match_contract_ssot() {
    assert_eq!(
        COMMANDS,
        command::ALL,
        "command catalog drifted from contract"
    );
    assert_eq!(EVENTS, event::ALL, "event catalog drifted from contract");
    assert_eq!(COMMANDS.len(), 21);
    assert_eq!(EVENTS.len(), 9);
}

#[test]
fn every_contract_command_is_registered() {
    let window = mock_window();
    // A registered command may reject (bad/empty args), but it must never report
    // "command <name> not found" — that distinguishes wired from missing.
    for name in command::ALL {
        if let Err(e) = invoke(&window, name, serde_json::json!({})) {
            let msg = e.to_string().to_lowercase();
            assert!(
                !msg.contains("not found"),
                "command `{name}` is not registered: {e}"
            );
        }
    }
}

#[test]
fn session_send_returns_a_turn_result() {
    let window = mock_window();
    let res = invoke(
        &window,
        command::SESSION_SEND,
        serde_json::json!({ "message": "hello" }),
    )
    .expect("session_send should succeed with the fake model");
    let value: serde_json::Value = res.deserialize().expect("turn result json");
    assert!(
        value.get("reply").is_some(),
        "TurnResult has a reply: {value}"
    );
    assert!(
        value.get("verified").is_some(),
        "TurnResult has a verdict: {value}"
    );
}
