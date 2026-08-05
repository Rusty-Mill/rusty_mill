// The tray app has no console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The menu bar app: a panel that is one keystroke away from anywhere.
//!
//! All the behaviour lives in `inventory-core`; this crate is the shell —
//! tray icon, global shortcuts, and a webview panel. Keeping it that thin is
//! what lets the same capabilities ship as `inv` on the terminal.

mod clipboard;

use inventory_core::{Inventory, Retention, SearchQuery, SourceId};
use serde::Serialize;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub struct AppState {
    inventory: Mutex<Inventory>,
}

type CmdResult<T> = Result<T, String>;

fn stringify(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// --- commands ---------------------------------------------------------------

#[derive(Serialize)]
struct SearchPayload {
    hits: Vec<inventory_core::SearchHit>,
    semantic_model: String,
    semantic_available: bool,
    total_candidates: usize,
}

#[tauri::command]
fn search(
    state: tauri::State<'_, AppState>,
    query: String,
    sources: Vec<String>,
    meaning: bool,
    limit: usize,
) -> CmdResult<SearchPayload> {
    let inv = state.inventory.lock().map_err(stringify)?;
    let mut q = SearchQuery::new(query);
    q.meaning = meaning;
    q.limit = limit.clamp(1, 100);
    q.sources = sources
        .iter()
        .filter_map(|s| s.parse::<SourceId>().ok())
        .collect();

    let response = inv.search(&q).map_err(stringify)?;
    Ok(SearchPayload {
        hits: response.hits,
        semantic_model: response.semantic_model,
        semantic_available: response.semantic_available,
        total_candidates: response.total_candidates,
    })
}

#[derive(Serialize)]
struct ConversationPayload {
    conversation: inventory_core::Conversation,
    messages: Vec<inventory_core::Message>,
}

#[tauri::command]
fn conversation(state: tauri::State<'_, AppState>, id: i64) -> CmdResult<ConversationPayload> {
    let inv = state.inventory.lock().map_err(stringify)?;
    let (conversation, messages) = inv.conversation(id).map_err(stringify)?;
    Ok(ConversationPayload {
        conversation,
        messages,
    })
}

#[tauri::command]
fn sources(state: tauri::State<'_, AppState>) -> CmdResult<Vec<inventory_core::SourceStatus>> {
    let inv = state.inventory.lock().map_err(stringify)?;
    inv.source_status().map_err(stringify)
}

#[derive(Serialize)]
struct StatsPayload {
    conversations: i64,
    messages: i64,
    index_bytes: u64,
    encrypted: bool,
    entropy: f64,
    retention: String,
    semantic_model: String,
    semantic_available: bool,
    scratchpad_enabled: bool,
    version: String,
    index_path: String,
}

#[tauri::command]
fn stats(state: tauri::State<'_, AppState>) -> CmdResult<StatsPayload> {
    let inv = state.inventory.lock().map_err(stringify)?;
    let s = inv.stats().map_err(stringify)?;
    Ok(StatsPayload {
        conversations: s.conversations,
        messages: s.messages,
        index_bytes: s.index_bytes,
        encrypted: s.encrypted,
        entropy: s.entropy_bits_per_byte,
        retention: s.retention.label().to_string(),
        semantic_model: s.embedding_model,
        semantic_available: s.semantic_available,
        scratchpad_enabled: s.scratchpad_enabled,
        version: inventory_core::VERSION.to_string(),
        index_path: inv.path().display().to_string(),
    })
}

#[derive(Serialize)]
struct CapturePayload {
    hits: Vec<inventory_core::SearchHit>,
    semantic_available: bool,
}

#[tauri::command]
fn capture(state: tauri::State<'_, AppState>, text: String) -> CmdResult<CapturePayload> {
    let inv = state.inventory.lock().map_err(stringify)?;
    let result = inv.capture(&text).map_err(stringify)?;
    Ok(CapturePayload {
        hits: result.related.hits,
        semantic_available: result.related.semantic_available,
    })
}

#[tauri::command]
fn clips(state: tauri::State<'_, AppState>, limit: usize) -> CmdResult<Vec<inventory_core::Clip>> {
    let inv = state.inventory.lock().map_err(stringify)?;
    inv.clips(limit).map_err(stringify)
}

#[tauri::command]
fn set_scratchpad(state: tauri::State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    let inv = state.inventory.lock().map_err(stringify)?;
    inv.set_scratchpad_enabled(enabled).map_err(stringify)
}

#[tauri::command]
fn clear_clips(state: tauri::State<'_, AppState>) -> CmdResult<usize> {
    let inv = state.inventory.lock().map_err(stringify)?;
    inv.clear_clips().map_err(stringify)
}

#[tauri::command]
fn export_clips(state: tauri::State<'_, AppState>) -> CmdResult<String> {
    let inv = state.inventory.lock().map_err(stringify)?;
    inv.export_clips().map_err(stringify)
}

#[tauri::command]
fn primer(state: tauri::State<'_, AppState>, id: i64) -> CmdResult<String> {
    let inv = state.inventory.lock().map_err(stringify)?;
    inv.primer(id).map_err(stringify)
}

#[derive(Serialize)]
struct ResumePayload {
    command: String,
    cwd: String,
    project_moved: bool,
    /// False when this source cannot be reopened from outside; the UI offers
    /// a primer instead.
    supported: bool,
    message: String,
}

#[tauri::command]
fn resume(state: tauri::State<'_, AppState>, id: i64) -> CmdResult<ResumePayload> {
    let inv = state.inventory.lock().map_err(stringify)?;
    match inv.resume(id) {
        Ok(cmd) => Ok(ResumePayload {
            command: cmd.display(),
            cwd: cmd.cwd.display().to_string(),
            project_moved: cmd.project_moved,
            supported: true,
            message: String::new(),
        }),
        Err(e) => Ok(ResumePayload {
            command: String::new(),
            cwd: String::new(),
            project_moved: false,
            supported: false,
            message: e.to_string(),
        }),
    }
}

#[derive(Serialize)]
struct RetentionPayload {
    slug: String,
    label: String,
    conversations: i64,
    bytes: i64,
    selected: bool,
}

#[tauri::command]
fn retention_options(state: tauri::State<'_, AppState>) -> CmdResult<Vec<RetentionPayload>> {
    let inv = state.inventory.lock().map_err(stringify)?;
    Ok(inv
        .retention_options()
        .map_err(stringify)?
        .into_iter()
        .map(|o| RetentionPayload {
            slug: o.retention.slug().to_string(),
            label: o.retention.label().to_string(),
            conversations: o.conversations,
            bytes: o.bytes,
            selected: o.selected,
        })
        .collect())
}

#[tauri::command]
fn set_retention(state: tauri::State<'_, AppState>, window: String) -> CmdResult<usize> {
    let retention: Retention = window.parse().map_err(stringify)?;
    let inv = state.inventory.lock().map_err(stringify)?;
    inv.set_retention(retention).map_err(stringify)
}

#[derive(Serialize)]
struct IndexPayload {
    added: usize,
    updated: usize,
    frozen: Vec<String>,
    elapsed_ms: u64,
}

#[tauri::command]
fn index_now(state: tauri::State<'_, AppState>, full: bool) -> CmdResult<IndexPayload> {
    let mut inv = state.inventory.lock().map_err(stringify)?;
    let report = inv.index(full).map_err(stringify)?;
    Ok(IndexPayload {
        added: report.total_added(),
        updated: report.total_updated(),
        frozen: report
            .frozen()
            .iter()
            .filter_map(|r| r.source.map(|s| s.display_name().to_string()))
            .collect(),
        elapsed_ms: report.elapsed_ms as u64,
    })
}

#[tauri::command]
fn hide_panel(window: tauri::Window) {
    let _ = window.hide();
}

// --- shell ------------------------------------------------------------------

const PANEL: &str = "panel";

/// Show the panel in a given mode. The three global shortcuts all land here;
/// the panel is a single window that swaps its own contents, which keeps
/// focus behaviour consistent across the three entry points.
fn show_panel(app: &tauri::AppHandle, mode: &str) {
    let window = match app.get_webview_window(PANEL) {
        Some(w) => w,
        None => {
            match WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("index.html".into()))
                .title("Inventory")
                .inner_size(760.0, 520.0)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .center()
                .visible(false)
                .build()
            {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("could not create the panel: {e}");
                    return;
                }
            }
        }
    };

    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit("panel-mode", mode);
}

fn register_shortcuts(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // ⌘⇧Space search · ⌘⇧N capture · ⌘⇧V scratchpad.
    let bindings = [
        (
            Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Space),
            "search",
        ),
        (
            Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyN),
            "capture",
        ),
        (
            Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyV),
            "scratchpad",
        ),
    ];

    for (shortcut, mode) in bindings {
        let handle = app.clone();
        let mode = mode.to_string();
        app.global_shortcut()
            .on_shortcut(shortcut, move |_app, _sc, event| {
                // Fire once per press, not again on release.
                if event.state() == ShortcutState::Pressed {
                    show_panel(&handle, &mode);
                }
            })?;
    }
    Ok(())
}

/// Menu bar presence: the app has no dock icon and no window of its own until
/// a shortcut summons one, so the tray is the only thing that proves it is
/// running.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let search = MenuItem::with_id(app, "search", "Search…", true, Some("Cmd+Shift+Space"))?;
    let capture = MenuItem::with_id(app, "capture", "Quick capture…", true, Some("Cmd+Shift+N"))?;
    let scratch = MenuItem::with_id(
        app,
        "scratchpad",
        "Clipboard scratchpad…",
        true,
        Some("Cmd+Shift+V"),
    )?;
    let palette = MenuItem::with_id(app, "palette", "Settings…", true, Some("Cmd+K"))?;
    let quit = MenuItem::with_id(app, "quit", "Quit Inventory", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&search, &capture, &scratch, &palette, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::Anyhow(anyhow_msg("the tray icon is missing from the bundle"))
        })?)
        .icon_as_template(true)
        .tooltip("Inventory — ⌘⇧Space to search")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            mode => show_panel(app, mode),
        })
        .build(app)?;
    Ok(())
}

fn anyhow_msg(msg: &'static str) -> anyhow::Error {
    anyhow::Error::msg(msg)
}

fn main() {
    let inventory = match Inventory::open() {
        Ok(inv) => inv,
        Err(e) => {
            // A key that cannot be read is fatal on purpose — see
            // `inventory_core::Error::KeyUnavailable`. Starting over would
            // discard an index that is still perfectly good.
            eprintln!("Inventory could not open its index.\n\n{e}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            inventory: Mutex::new(inventory),
        })
        .invoke_handler(tauri::generate_handler![
            search,
            conversation,
            sources,
            stats,
            capture,
            clips,
            set_scratchpad,
            clear_clips,
            export_clips,
            primer,
            resume,
            retention_options,
            set_retention,
            index_now,
            hide_panel,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // A shortcut already claimed by another app must not stop the app
            // starting — the tray and `inv` still reach everything.
            if let Err(e) = register_shortcuts(&handle) {
                eprintln!(
                    "could not register a global shortcut ({e}); \
                     use the tray icon or the `inv` command instead"
                );
            }
            build_tray(&handle)?;
            clipboard::spawn_watcher(handle.clone());

            // `--show` opens the panel immediately, which is how you confirm
            // an install works without guessing at the shortcut.
            if std::env::args().any(|a| a == "--show") {
                show_panel(&handle, "search");
            }

            // First pass in the background: "First pass takes seconds. After
            // that it stays live as you work."
            std::thread::spawn(move || {
                if let Some(state) = handle.try_state::<AppState>() {
                    if let Ok(mut inv) = state.inventory.lock() {
                        if let Err(e) = inv.index(false) {
                            eprintln!("initial index failed: {e}");
                        }
                    }
                }
                let _ = handle.emit("index-complete", ());
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start Inventory");
}
