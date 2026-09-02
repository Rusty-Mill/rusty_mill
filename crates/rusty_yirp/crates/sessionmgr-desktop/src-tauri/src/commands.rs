//! Tauri commands: the frontend's entire surface onto the daemon. Each
//! one is a thin translation from JS-friendly primitives (`String` ids,
//! not `SessionId`; errors as `String`, not this crate's own error
//! shape -- Tauri requires `Result::Err` to `Serialize`) into a
//! `client.rs`/`attach.rs` call.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use sessionmgr_protocol::{
    AgentKind, ChangedFile, Request, Response, SessionId, SessionKind, SessionSummary,
};
use tauri::{AppHandle, State};

use crate::attach::{self, AttachHandle};
use crate::client;

pub struct AppState {
    pub socket: PathBuf,
    pub attaches: Mutex<HashMap<String, AttachHandle>>,
}

fn parse_id(id: &str) -> Result<SessionId, String> {
    id.parse()
        .map_err(|e| format!("invalid session id `{id}`: {e}"))
}

/// Parses the palette's free-text agent name. Duplicated from
/// `sessionmgr-daemon::lib::parse_agent_name` (same three names, same
/// error shape) for the reason `paths.rs`'s module docs already give:
/// this crate cannot depend on `sessionmgr-daemon`.
fn parse_agent_name(name: &str) -> Result<AgentKind, String> {
    match name {
        "claude" | "claude-code" => Ok(AgentKind::ClaudeCode),
        "codex" => Ok(AgentKind::Codex),
        "gemini" => Ok(AgentKind::Gemini),
        other => Err(format!(
            "unknown agent `{other}` (expected `claude`, `codex`, or `gemini`)"
        )),
    }
}

#[tauri::command]
pub fn session_list(state: State<AppState>) -> Result<Vec<SessionSummary>, String> {
    let response = client::request(&state.socket, &Request::SessionList)?;
    client::expect(response, |r| match r {
        Response::Sessions { sessions } => Some(sessions),
        _ => None,
    })
}

/// Creates a plain worktree session against `repo`, with no agent and
/// this platform's default shell unless `agent` is given -- the same
/// "fast shortcut for the common case" scope
/// `sessionmgr-tui::client::session_new`'s own doc comment describes.
#[tauri::command]
pub fn session_new(
    state: State<AppState>,
    repo: String,
    agent: Option<String>,
) -> Result<String, String> {
    let agent = agent.map(|a| parse_agent_name(&a)).transpose()?;
    let response = client::request(
        &state.socket,
        &Request::SessionNew {
            kind: SessionKind::Worktree,
            command: Vec::new(),
            repo: Some(PathBuf::from(repo)),
            pty: true,
            agent,
            hooks: false,
            parent: None,
            wait_for_parent: false,
        },
    )?;
    client::expect(response, |r| match r {
        Response::SessionCreated { id } => Some(id.to_string()),
        _ => None,
    })
}

#[tauri::command]
pub fn session_close(state: State<AppState>, id: String) -> Result<(), String> {
    let id = parse_id(&id)?;
    let response = client::request(
        &state.socket,
        &Request::SessionClose {
            id,
            disposition: None,
        },
    )?;
    client::expect(response, |r| matches!(r, Response::Ok).then_some(()))
}

#[tauri::command]
pub fn session_rename(
    state: State<AppState>,
    id: String,
    name: Option<String>,
) -> Result<(), String> {
    let id = parse_id(&id)?;
    let response = client::request(&state.socket, &Request::SessionRename { id, name })?;
    client::expect(response, |r| matches!(r, Response::Ok).then_some(()))
}

#[tauri::command]
pub fn session_fork(state: State<AppState>, id: String) -> Result<String, String> {
    let id = parse_id(&id)?;
    let response = client::request(&state.socket, &Request::SessionFork { id, pty: true })?;
    client::expect(response, |r| match r {
        Response::SessionCreated { id } => Some(id.to_string()),
        _ => None,
    })
}

#[tauri::command]
pub fn session_switch_agent(
    state: State<AppState>,
    id: String,
    agent: String,
) -> Result<String, String> {
    let id = parse_id(&id)?;
    let agent = parse_agent_name(&agent)?;
    let response = client::request(
        &state.socket,
        &Request::SessionSwitchAgent {
            id,
            agent,
            pty: true,
        },
    )?;
    client::expect(response, |r| match r {
        Response::SessionCreated { id } => Some(id.to_string()),
        _ => None,
    })
}

#[tauri::command]
pub fn git_status(state: State<AppState>, id: String) -> Result<Vec<ChangedFile>, String> {
    let id = parse_id(&id)?;
    let response = client::request(&state.socket, &Request::GitStatus { id })?;
    client::expect(response, |r| match r {
        Response::GitStatus { files } => Some(files),
        _ => None,
    })
}

#[tauri::command]
pub fn git_diff(
    state: State<AppState>,
    id: String,
    path: Option<String>,
) -> Result<String, String> {
    let id = parse_id(&id)?;
    let response = client::request(&state.socket, &Request::GitDiff { id, path })?;
    client::expect(response, |r| match r {
        Response::GitDiff { diff } => Some(diff),
        _ => None,
    })
}

/// Opens (or, if already open, no-ops on) a live attach connection for
/// `id`, streaming its output to the frontend as `"session-event"`
/// events from here on.
#[tauri::command]
pub fn attach_session(app: AppHandle, state: State<AppState>, id: String) -> Result<(), String> {
    let mut attaches = state
        .attaches
        .lock()
        .map_err(|_| "attach registry poisoned".to_owned())?;
    if attaches.contains_key(&id) {
        return Ok(());
    }
    let parsed = parse_id(&id)?;
    let handle = attach::start(&state.socket, parsed, app)?;
    attaches.insert(id, handle);
    Ok(())
}

/// Closes a pane's attach connection -- does not close the session
/// itself, matching `session_close`'s own separate, explicit command.
#[tauri::command]
pub fn detach_session(state: State<AppState>, id: String) -> Result<(), String> {
    let mut attaches = state
        .attaches
        .lock()
        .map_err(|_| "attach registry poisoned".to_owned())?;
    if let Some(handle) = attaches.remove(&id) {
        handle.close();
    }
    Ok(())
}

#[tauri::command]
pub fn send_input(state: State<AppState>, id: String, data: Vec<u8>) -> Result<(), String> {
    let parsed = parse_id(&id)?;
    let mut attaches = state
        .attaches
        .lock()
        .map_err(|_| "attach registry poisoned".to_owned())?;
    let handle = attaches
        .get_mut(&id)
        .ok_or_else(|| format!("{id} is not attached"))?;
    handle.send_input(parsed, data)
}

#[tauri::command]
pub fn send_resize(state: State<AppState>, id: String, rows: u16, cols: u16) -> Result<(), String> {
    let parsed = parse_id(&id)?;
    let mut attaches = state
        .attaches
        .lock()
        .map_err(|_| "attach registry poisoned".to_owned())?;
    let handle = attaches
        .get_mut(&id)
        .ok_or_else(|| format!("{id} is not attached"))?;
    handle.send_resize(parsed, rows, cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_name_accepts_every_known_agent() {
        assert_eq!(parse_agent_name("claude"), Ok(AgentKind::ClaudeCode));
        assert_eq!(parse_agent_name("claude-code"), Ok(AgentKind::ClaudeCode));
        assert_eq!(parse_agent_name("codex"), Ok(AgentKind::Codex));
        assert_eq!(parse_agent_name("gemini"), Ok(AgentKind::Gemini));
    }

    #[test]
    fn parse_agent_name_rejects_an_unknown_name() {
        assert!(parse_agent_name("gpt5").is_err());
        assert!(parse_agent_name("").is_err());
    }

    #[test]
    fn parse_id_rejects_a_malformed_id_before_it_reaches_the_daemon() {
        assert!(parse_id("not-a-real-id").is_err());
    }
}
