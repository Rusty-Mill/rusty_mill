//! Ai Env Manager desktop shell (pack D31, D32). A view over `ai_env_app`:
//! three typed commands (`list`, `plan`, `apply`) and no file, shell or HTTP
//! access for the page. The skill folders and tools are fixed at launch from
//! the command line (`--skills DIR`, `--tool ID`, like `aienv`), so the page
//! cannot point the app at other folders. `apply` plans again on the backend
//! and never trusts a plan sent from the page.

pub mod view;

use ai_env_app::{
    apply, find_entry, home_dir, load_entries, observe, plan, select_tools, Op, Scope,
};
use ai_env_core::{EntryId, Tool, ToolId};
use std::ffi::OsString;
use std::path::PathBuf;
use view::{ListView, Skipped, ToolPlan, ToolResult};

/// Fixed at launch; what the commands act on.
pub struct Ctx {
    scope: Scope,
    home: PathBuf,
}

const USAGE: &str = "usage: ai-env-gui --skills DIR... [--tool ID]...";

/// Parses `--skills DIR` (repeatable, required) and `--tool ID` (repeatable).
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
    if skills.is_empty() {
        return Err(format!("--skills is required\n{USAGE}"));
    }
    Ok(Scope { skills, tools })
}

fn op(enable: bool) -> Op {
    if enable {
        Op::Enable
    } else {
        Op::Disable
    }
}

impl Ctx {
    pub fn new(scope: Scope, home: PathBuf) -> Self {
        Ctx { scope, home }
    }

    fn tools(&self) -> Result<Vec<Tool>, String> {
        select_tools(&self.scope, &self.home).map_err(|e| e.to_string())
    }

    pub fn list(&self) -> Result<ListView, String> {
        let tools = self.tools()?;
        let loaded = load_entries(&self.scope).map_err(|e| e.to_string())?;
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
        let loaded = load_entries(&self.scope).map_err(|e| e.to_string())?;
        find_entry(&loaded.entries, &id)
            .cloned()
            .map_err(|e| e.to_string())
    }

    /// Preview only: reads the disk, writes nothing.
    pub fn plan(&self, entry: &str, enable: bool) -> Result<Vec<ToolPlan>, String> {
        let entry = self.entry(entry)?;
        self.tools()?
            .iter()
            .map(|t| {
                let p = plan(&entry, t, op(enable)).map_err(|e| e.to_string())?;
                Ok(view::tool_plan(t, &p))
            })
            .collect()
    }

    /// Plans again and applies each tool whose plan has no refusal, like
    /// `aienv --yes`. A failure partway names the tools already changed.
    pub fn apply(&self, entry: &str, enable: bool) -> Result<Vec<ToolResult>, String> {
        let entry = self.entry(entry)?;
        let mut done: Vec<ToolResult> = Vec::new();
        for tool in self.tools()? {
            let p = plan(&entry, &tool, op(enable)).map_err(|e| e.to_string())?;
            let outcome = if p.is_noop() {
                "nothing"
            } else if p.has_refusals() {
                "refused"
            } else {
                if let Err(e) = apply(&p) {
                    let changed: Vec<&str> = done
                        .iter()
                        .filter(|r| r.outcome == "applied")
                        .map(|r| r.tool.as_str())
                        .collect();
                    return Err(format!("{}: {e} (already changed: {changed:?})", tool.id));
                }
                "applied"
            };
            done.push(ToolResult {
                tool: tool.id.to_string(),
                outcome,
            });
        }
        Ok(done)
    }
}

#[tauri::command]
fn list(ctx: tauri::State<Ctx>) -> Result<ListView, String> {
    ctx.list()
}

#[tauri::command]
fn plan_change(
    ctx: tauri::State<Ctx>,
    entry: String,
    enable: bool,
) -> Result<Vec<ToolPlan>, String> {
    ctx.plan(&entry, enable)
}

#[tauri::command]
fn apply_change(
    ctx: tauri::State<Ctx>,
    entry: String,
    enable: bool,
) -> Result<Vec<ToolResult>, String> {
    ctx.apply(&entry, enable)
}

/// Opens the window. `args` are the command-line arguments after the program.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let scope = parse_scope(args)?;
    let home =
        home_dir().ok_or("set HOME (or USERPROFILE) so ~/ in the tool catalog can be resolved")?;
    tauri::Builder::default()
        .manage(Ctx::new(scope, home))
        .invoke_handler(tauri::generate_handler![list, plan_change, apply_change])
        .run(tauri::generate_context!())
        .map_err(|e| e.to_string())
}
