//! Ai Env Manager desktop shell (pack D31, D32). A view over `ai_env_app`:
//! typed commands (`list`, `plan`, `apply`, and the editor's `files`, `file`,
//! `preview_edit`, `apply_edit`, `preview_push`, `apply_push`) and no file, shell or HTTP access for the page.
//! The editor takes an entry id and a path relative to it, never an absolute
//! path; every rule is enforced in `ai_env_app`. The skill folders and tools are fixed at launch from
//! the command line (`--skills DIR`, `--tool ID`, like `aienv`), so the page
//! cannot point the app at other folders. `apply` plans again on the backend
//! and never trusts a plan sent from the page.

pub mod view;

use ai_env_app::{
    apply, edit_apply, edit_preview, find_entry, home_dir, list_files, load_entries, observe, plan,
    push_apply, push_preview, read_file, select_tools, Place, Scope,
};
use ai_env_core::{EntryId, Tool, ToolId};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Mutex;
use view::{
    BaseIn, FileView, FilesView, ListView, Outcome, PreviewView, PushPreviewView, PushedView,
    Skipped, ToolPlan, ToolResult, Verb,
};

/// What the commands act on: the tools are fixed at launch; the skill folders
/// start from the command line and grow only through the native folder picker
/// (D46), never from a path the page sends.
pub struct Ctx {
    scope: Mutex<Scope>,
    home: PathBuf,
}

const USAGE: &str = "usage: ai-env-gui [--skills DIR]... [--tool ID]...";

/// Parses `--skills DIR` and `--tool ID`, both repeatable and optional (folders
/// can be chosen in the window).
pub fn parse_scope(args: impl IntoIterator<Item = OsString>) -> Result<Scope, String> {
    let (mut skills, mut tools) = (Vec::new(), Vec::new());
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("{flag:?} needs a value\n{USAGE}"))?;
        match flag.to_str() {
            Some("--skills") => skills.push(PathBuf::from(value)),
            Some("--tool") => {
                let id = value.to_str().ok_or("--tool must be valid UTF-8")?;
                tools.push(
                    id.parse::<ToolId>()
                        .map_err(|e| format!("--tool {id:?}: {e}"))?,
                );
            }
            _ => return Err(format!("unexpected argument {flag:?}\n{USAGE}")),
        }
    }
    Ok(Scope { skills, tools })
}

impl Ctx {
    pub fn new(scope: Scope, home: PathBuf) -> Self {
        Ctx {
            scope: Mutex::new(scope),
            home,
        }
    }

    fn scope(&self) -> Scope {
        self.scope.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Adds a skills folder to this session (not saved, D47). The folder must
    /// be readable now; one already present is not added twice.
    pub fn add_folder(&self, dir: PathBuf) -> Result<(), String> {
        ai_env_app::load_entries(&Scope {
            skills: vec![dir.clone()],
            tools: Vec::new(),
        })
        .map_err(|e| e.to_string())?;
        let mut scope = self.scope.lock().unwrap_or_else(|e| e.into_inner());
        if !scope.skills.contains(&dir) {
            scope.skills.push(dir);
        }
        Ok(())
    }

    fn tools(&self) -> Result<Vec<Tool>, String> {
        select_tools(&self.scope(), &self.home).map_err(|e| e.to_string())
    }

    pub fn list(&self) -> Result<ListView, String> {
        let tools = self.tools()?;
        let loaded = load_entries(&self.scope()).map_err(|e| e.to_string())?;
        let mut rows = Vec::new();
        for entry in &loaded.entries {
            let mut cells = Vec::new();
            for tool in &tools {
                cells.push((tool, observe(entry, tool).map_err(|e| e.to_string())?));
            }
            rows.push(view::row(entry, cells));
        }
        Ok(ListView {
            tools: tools.iter().map(|t| t.id.to_string()).collect(),
            rows,
            skipped: loaded
                .skipped
                .into_iter()
                .map(|s| Skipped {
                    path: s.path.display().to_string(),
                    reason: s.reason,
                })
                .collect(),
        })
    }

    fn entry(&self, id: &str) -> Result<ai_env_core::Entry, String> {
        let id: EntryId = id.parse().map_err(|e| format!("{id:?}: {e}"))?;
        let loaded = load_entries(&self.scope()).map_err(|e| e.to_string())?;
        find_entry(&loaded.entries, &id)
            .cloned()
            .map_err(|e| e.to_string())
    }

    /// Preview only: reads the disk, writes nothing.
    pub fn plan(&self, entry: &str, verb: Verb) -> Result<Vec<ToolPlan>, String> {
        let entry = self.entry(entry)?;
        self.tools()?
            .iter()
            .map(|t| {
                let p = plan(&entry, t, verb.into()).map_err(|e| e.to_string())?;
                Ok(view::tool_plan(t, &p))
            })
            .collect()
    }

    /// Plans again and applies each tool whose plan has no refusal, like
    /// `aienv --yes`. A failure partway names the tools already changed.
    pub fn apply(&self, entry: &str, verb: Verb) -> Result<Vec<ToolResult>, String> {
        let entry = self.entry(entry)?;
        let mut done: Vec<ToolResult> = Vec::new();
        for tool in self.tools()? {
            let p = plan(&entry, &tool, verb.into()).map_err(|e| e.to_string())?;
            let outcome = if p.is_noop() {
                Outcome::Nothing
            } else if p.has_refusals() {
                Outcome::Refused
            } else {
                if let Err(e) = apply(&p) {
                    let changed: Vec<&str> = done
                        .iter()
                        .filter(|r| r.outcome == Outcome::Applied)
                        .map(|r| r.tool.as_str())
                        .collect();
                    return Err(format!("{}: {e} (already changed: {changed:?})", tool.id));
                }
                Outcome::Applied
            };
            done.push(ToolResult {
                tool: tool.id.to_string(),
                outcome,
            });
        }
        Ok(done)
    }
}

/// What the editor commands act on: the entry's source folder, or what one
/// tool has of it. `Err` for a tool id that does not parse.
fn place(tool: Option<&str>) -> Result<Place, String> {
    match tool {
        None => Ok(Place::Source),
        Some(id) => id
            .parse::<ToolId>()
            .map(Place::Tool)
            .map_err(|e| format!("{id:?}: {e}")),
    }
}

impl Ctx {
    /// The files the editor can name in `entry`, and which may be changed.
    pub fn files(&self, entry: &str, tool: Option<&str>) -> Result<FilesView, String> {
        let (entry, tools) = (self.entry(entry)?, self.tools()?);
        list_files(&entry, &place(tool)?, &tools)
            .map(view::files)
            .map_err(|e| e.to_string())
    }

    /// One file's text and the hash to send back when saving.
    pub fn file(&self, entry: &str, tool: Option<&str>, path: &str) -> Result<FileView, String> {
        let (entry, tools) = (self.entry(entry)?, self.tools()?);
        read_file(&entry, &place(tool)?, &tools, path)
            .map(view::file)
            .map_err(|e| e.to_string())
    }

    /// Preview only: the diff a save would make. Writes nothing.
    pub fn edit_preview(
        &self,
        entry: &str,
        tool: Option<&str>,
        path: &str,
        text: String,
        hash: &str,
    ) -> Result<PreviewView, String> {
        let (entry, tools) = (self.entry(entry)?, self.tools()?);
        edit_preview(&entry, &place(tool)?, &tools, path, text, hash)
            .map(view::preview)
            .map_err(|e| e.to_string())
    }

    /// Saves. Every rule is checked again here from these inputs, so nothing
    /// from the preview is trusted.
    pub fn edit_apply(
        &self,
        entry: &str,
        tool: Option<&str>,
        path: &str,
        text: String,
        hash: &str,
    ) -> Result<(), String> {
        let (entry, tools) = (self.entry(entry)?, self.tools()?);
        edit_apply(&entry, &place(tool)?, &tools, path, text, hash).map_err(|e| e.to_string())
    }

    /// Preview only: what pushing `tool`'s edited copy back to the source
    /// would change (one `path`, else every changed file). Writes nothing.
    pub fn push_preview(
        &self,
        entry: &str,
        tool: &str,
        path: Option<&str>,
    ) -> Result<PushPreviewView, String> {
        let (entry, tools) = (self.entry(entry)?, self.tools()?);
        let tool = tool
            .parse::<ToolId>()
            .map_err(|e| format!("{tool:?}: {e}"))?;
        push_preview(&entry, &tool, &tools, path)
            .map(view::push_preview)
            .map_err(|e| e.to_string())
    }

    /// Pushes. Both sides are re-checked against the hashes the preview
    /// showed, so a file changed since is refused.
    pub fn push_apply(
        &self,
        entry: &str,
        tool: &str,
        bases: Vec<BaseIn>,
    ) -> Result<PushedView, String> {
        let (entry, tools) = (self.entry(entry)?, self.tools()?);
        let tool = tool
            .parse::<ToolId>()
            .map_err(|e| format!("{tool:?}: {e}"))?;
        let bases: Vec<_> = bases.into_iter().map(Into::into).collect();
        push_apply(&entry, &tool, &tools, &bases)
            .map(view::pushed)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn list(ctx: tauri::State<Ctx>) -> Result<ListView, String> {
    ctx.list()
}

#[tauri::command]
fn plan_change(ctx: tauri::State<Ctx>, entry: String, verb: Verb) -> Result<Vec<ToolPlan>, String> {
    ctx.plan(&entry, verb)
}

#[tauri::command]
fn apply_change(
    ctx: tauri::State<Ctx>,
    entry: String,
    verb: Verb,
) -> Result<Vec<ToolResult>, String> {
    ctx.apply(&entry, verb)
}

#[tauri::command]
fn files(ctx: tauri::State<Ctx>, entry: String, tool: Option<String>) -> Result<FilesView, String> {
    ctx.files(&entry, tool.as_deref())
}

#[tauri::command]
fn file(
    ctx: tauri::State<Ctx>,
    entry: String,
    tool: Option<String>,
    path: String,
) -> Result<FileView, String> {
    ctx.file(&entry, tool.as_deref(), &path)
}

#[tauri::command]
fn preview_edit(
    ctx: tauri::State<Ctx>,
    entry: String,
    tool: Option<String>,
    path: String,
    text: String,
    hash: String,
) -> Result<PreviewView, String> {
    ctx.edit_preview(&entry, tool.as_deref(), &path, text, &hash)
}

#[tauri::command]
fn apply_edit(
    ctx: tauri::State<Ctx>,
    entry: String,
    tool: Option<String>,
    path: String,
    text: String,
    hash: String,
) -> Result<(), String> {
    ctx.edit_apply(&entry, tool.as_deref(), &path, text, &hash)
}

#[tauri::command]
fn preview_push(
    ctx: tauri::State<Ctx>,
    entry: String,
    tool: String,
    path: Option<String>,
) -> Result<PushPreviewView, String> {
    ctx.push_preview(&entry, &tool, path.as_deref())
}

#[tauri::command]
fn apply_push(
    ctx: tauri::State<Ctx>,
    entry: String,
    tool: String,
    bases: Vec<BaseIn>,
) -> Result<PushedView, String> {
    ctx.push_apply(&entry, &tool, bases)
}

/// Opens the native folder dialog and adds the folder picked. The page only
/// triggers it: the path never passes through the page. `Ok(false)` means the
/// dialog was cancelled.
#[tauri::command]
async fn choose_folder(app: tauri::AppHandle, ctx: tauri::State<'_, Ctx>) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(picked) = app.dialog().file().blocking_pick_folder() else {
        return Ok(false);
    };
    let dir = picked.into_path().map_err(|e| e.to_string())?;
    ctx.add_folder(dir)?;
    Ok(true)
}

/// Opens the window. `args` are the command-line arguments after the program.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let scope = parse_scope(args)?;
    let home =
        home_dir().ok_or("set HOME (or USERPROFILE) so ~/ in the tool catalog can be resolved")?;
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Ctx::new(scope, home))
        .invoke_handler(tauri::generate_handler![
            list,
            plan_change,
            apply_change,
            files,
            file,
            preview_edit,
            apply_edit,
            preview_push,
            apply_push,
            choose_folder
        ])
        .run(tauri::generate_context!())
        .map_err(|e| e.to_string())
}
