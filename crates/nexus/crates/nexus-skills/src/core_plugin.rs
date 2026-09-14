//! Core plugin wrapping [`SkillRegistry`].
//!
//! Exposes the registry over kernel IPC so the agent planner + Chat
//! panel + future Workflow system can consult available skills
//! without linking `nexus-skills` directly. Plugin is *load-only* —
//! mutations happen by editing `.skill.md` files and calling
//! `reload`, not by writing through IPC.
//!
//! # Handlers
//!
//! | Id | Command            | Args               | Purpose                                  |
//! |---:|--------------------|--------------------|------------------------------------------|
//! | 1  | `list`             | `{}`               | Every loaded skill                       |
//! | 2  | `get`              | `{ id }`           | One skill by id (404 if missing)         |
//! | 3  | `list_by_context`  | `{ context }`      | Skills whose `applicable_contexts` match |
//! | 4  | `triggered_by`     | `{ text }`         | Skills whose trigger matches `text`      |
//! | 5  | `reload`           | `{}`               | Re-scan the `<forge>/.forge/skills` dir  |
//! | 6  | `render`           | `{ id, values? }`  | Render a skill's body with parameter substitution |
//! | 7  | `compose`          | `{ id }`           | BL-021 — resolve `depends_on` closure into ordered fragments + merged body |
//! | 8  | `invoke`           | `{ skill_id, input, archetype? }` | BL-054 Phase 3 — run a skill via `com.nexus.agent::session_run` |
//!
//! Ids are append-only.
//!
//! # Intentional runtime cycle with `com.nexus.agent`
//!
//! Handler #8 (`invoke`) calls `com.nexus.agent::session_run` to
//! actually execute the skill, while the agent plugin calls
//! `com.nexus.skills::{triggered_by, compose, render}` during its
//! planning loop. Both plugins must be present for either to fully
//! function, but the calls are async and lock-free, so the cycle is
//! functional — not a deadlock. Boot order loads skills (#8) before
//! agent (#16); the load-time half of the cycle is therefore broken,
//! and only the runtime half remains. The mirror of this comment
//! lives in `crates/nexus-agent/src/core_plugin.rs`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexus_kernel::{Ipc as _, KernelPluginContext};
use nexus_plugins::{CorePlugin, CorePluginFuture, PluginError};
use serde::{Deserialize, Serialize};

#[cfg(feature = "ts-export")]
use schemars::JsonSchema;
#[cfg(feature = "ts-export")]
use ts_rs::TS;

use crate::{registry_index, SkillRegistry, SkillRegistryError};

// ── IPC arg types (audit P1-3 #113 — lifted from inline) ─────────────────────

/// Args for `com.nexus.skills::get` (handler id `2`). Lifted from an
/// inline `struct Args` inside [`SkillsCorePlugin::dispatch_get`] by
/// audit-2026-05-01 P1-3 (#113) so the schema generator can see the
/// shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS, JsonSchema))]
#[cfg_attr(
    feature = "ts-export",
    ts(
        export,
        export_to = "../../../packages/nexus-extension-api/src/generated/ipc/"
    )
)]
#[serde(deny_unknown_fields)]
pub struct GetSkillArgs {
    /// Unique kebab-case skill id to fetch.
    pub id: String,
}

/// Args for `com.nexus.skills::list_by_context` (handler id `3`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS, JsonSchema))]
#[cfg_attr(
    feature = "ts-export",
    ts(
        export,
        export_to = "../../../packages/nexus-extension-api/src/generated/ipc/"
    )
)]
#[serde(deny_unknown_fields)]
pub struct ListByContextArgs {
    /// Activation context — `pull-request`, `terminal`, `editor`,
    /// `ai-chat`, `agent`. Matched against each skill's
    /// `applicable_contexts` list.
    pub context: String,
}

/// Args for `com.nexus.skills::triggered_by` (handler id `4`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS, JsonSchema))]
#[cfg_attr(
    feature = "ts-export",
    ts(
        export,
        export_to = "../../../packages/nexus-extension-api/src/generated/ipc/"
    )
)]
#[serde(deny_unknown_fields)]
pub struct TriggeredByArgs {
    /// Text to scan for trigger keywords/phrases.
    pub text: String,
}

/// Args for `com.nexus.skills::render` (handler id `6`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS, JsonSchema))]
#[cfg_attr(
    feature = "ts-export",
    ts(
        export,
        export_to = "../../../packages/nexus-extension-api/src/generated/ipc/"
    )
)]
#[serde(deny_unknown_fields)]
pub struct RenderSkillArgs {
    /// Skill id to render.
    pub id: String,
    /// Parameter overrides for substitution. Each value is JSON
    /// (round-tripped to YAML internally so enum-comparison matches
    /// the skill's declared `values:` list).
    #[serde(default)]
    #[cfg_attr(feature = "ts-export", ts(type = "unknown"))]
    pub values: HashMap<String, serde_json::Value>,
}

/// Args for `com.nexus.skills::compose` (handler id `7`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS, JsonSchema))]
#[cfg_attr(
    feature = "ts-export",
    ts(
        export,
        export_to = "../../../packages/nexus-extension-api/src/generated/ipc/"
    )
)]
#[serde(deny_unknown_fields)]
pub struct ComposeSkillArgs {
    /// Skill id to compose. The engine resolves the `depends_on`
    /// closure transitively.
    pub id: String,
}

/// Args for `com.nexus.skills::invoke` (handler id `8`, BL-054 Phase 3).
///
/// Runs a skill by composing its `depends_on` closure into a merged
/// system prompt, then dispatching `com.nexus.agent::session_run`
/// with the user-supplied `input` as the goal. The reply is the agent
/// observation JSON returned by `session_run` verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS, JsonSchema))]
#[cfg_attr(
    feature = "ts-export",
    ts(
        export,
        export_to = "../../../packages/nexus-extension-api/src/generated/ipc/"
    )
)]
#[serde(deny_unknown_fields)]
pub struct InvokeSkillArgs {
    /// The skill id to run. Must match a `.skill.md` file's
    /// frontmatter `id`.
    pub skill_id: String,
    /// User-supplied goal text — passed to the agent as the `goal`
    /// argument.
    pub input: String,
    /// Optional archetype override. When omitted, defaults to
    /// `"general"` (matches the BL-054 Phase 3 spec — skills don't
    /// carry their own archetype today).
    #[serde(default)]
    pub archetype: Option<String>,
}

/// Reverse-DNS identifier.
pub const PLUGIN_ID: &str = "com.nexus.skills";

/// `list` handler id.
pub const HANDLER_LIST: u32 = 1;
/// `get` handler id.
pub const HANDLER_GET: u32 = 2;
/// `list_by_context` handler id.
pub const HANDLER_LIST_BY_CONTEXT: u32 = 3;
/// `triggered_by` handler id.
pub const HANDLER_TRIGGERED_BY: u32 = 4;
/// `reload` handler id.
pub const HANDLER_RELOAD: u32 = 5;
/// `render` handler id.
pub const HANDLER_RENDER: u32 = 6;
/// `compose` handler id (BL-021).
pub const HANDLER_COMPOSE: u32 = 7;
/// `invoke` handler id (BL-054 Phase 3).
pub const HANDLER_INVOKE: u32 = 8;

/// SD-06 — single source of truth for `(command-name, handler-id)`
/// pairs consumed by `nexus_bootstrap::plugins::skills::register`.
/// Order matches the pre-SD-06 bootstrap registration.
pub const IPC_HANDLERS: &[(&str, u32)] = &[
    ("list", HANDLER_LIST),
    ("get", HANDLER_GET),
    ("list_by_context", HANDLER_LIST_BY_CONTEXT),
    ("triggered_by", HANDLER_TRIGGERED_BY),
    ("reload", HANDLER_RELOAD),
    ("render", HANDLER_RENDER),
    ("compose", HANDLER_COMPOSE),
    ("invoke", HANDLER_INVOKE),
];

/// BL-054 Phase 3 — total budget for the agent invocation. The agent
/// session itself enforces a finer-grained per-round timeout; this is
/// the outer cap so the skills handler can't hang indefinitely on a
/// stuck provider.
const INVOKE_AGENT_TIMEOUT: Duration = Duration::from_secs(120);

/// Default archetype the BL-054 Phase 3 spec calls for when neither
/// the caller nor (future) skill metadata supplies one.
const DEFAULT_ARCHETYPE: &str = "general";

/// Core plugin — holds the skills root path + an in-memory registry
/// behind a mutex so dispatches stay `Send + Sync`.
pub struct SkillsCorePlugin {
    root: PathBuf,
    registry: Mutex<SkillRegistry>,
    /// BL-054 Phase 3 — kernel context, captured by `wire_context`.
    /// Required by `dispatch_async` for the `invoke` handler so it can
    /// reach `com.nexus.agent::session_run`. None means the plugin was
    /// loaded without the bootstrap doing the wiring (e.g. unit tests).
    context: Option<Arc<KernelPluginContext>>,
}

impl SkillsCorePlugin {
    /// Construct with the forge's `.forge/skills` directory.
    /// Eagerly loads the registry; partial parse failures are
    /// logged at `warn` and the registry starts with whatever did
    /// parse. Callers that want stricter startup can call `reload`
    /// themselves and inspect the error.
    #[must_use]
    pub fn open(skills_dir: PathBuf) -> Self {
        let registry = match SkillRegistry::load(&skills_dir) {
            Ok(reg) => reg,
            Err(SkillRegistryError::PartialParseFailure { count, first }) => {
                tracing::warn!(
                    path = %skills_dir.display(),
                    count,
                    first = %first,
                    "com.nexus.skills: {count} skill file(s) failed to parse during load"
                );
                SkillRegistry::empty()
            }
            Err(err) => {
                tracing::warn!(
                    path = %skills_dir.display(),
                    err = %err,
                    "com.nexus.skills: load failed; registry starts empty"
                );
                SkillRegistry::empty()
            }
        };
        // Best-effort: persist the on-disk REGISTRY.json index so
        // external CLIs can cold-start without a directory walk.
        // PRD-13 §3.1. Failures must not block plugin open.
        let index_path = skills_dir.join("REGISTRY.json");
        if let Err(err) = registry_index::write_index(&index_path, &skills_dir, &registry) {
            tracing::warn!(
                path = %index_path.display(),
                err = %err,
                "com.nexus.skills: failed to persist REGISTRY.json on open"
            );
        }
        Self {
            root: skills_dir,
            registry: Mutex::new(registry),
            context: None,
        }
    }
}

impl CorePlugin for SkillsCorePlugin {
    fn dispatch(
        &mut self,
        handler_id: u32,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        match handler_id {
            HANDLER_LIST => self.dispatch_list(),
            HANDLER_GET => self.dispatch_get(args),
            HANDLER_LIST_BY_CONTEXT => self.dispatch_list_by_context(args),
            HANDLER_TRIGGERED_BY => self.dispatch_triggered_by(args),
            HANDLER_RELOAD => self.dispatch_reload(),
            HANDLER_RENDER => self.dispatch_render(args),
            HANDLER_COMPOSE => self.dispatch_compose(args),
            // BL-054 Phase 3 — `invoke` is async (issues a nested
            // `com.nexus.agent` IPC call). Surface the routing mistake
            // via the typed `HandlerIsAsyncOnly` error.
            HANDLER_INVOKE => Err(PluginError::HandlerIsAsyncOnly {
                handler_id: HANDLER_INVOKE,
            }),
            other => Err(exec_err(format!("unknown handler id {other}"))),
        }
    }

    fn dispatch_async(
        &mut self,
        handler_id: u32,
        args: &serde_json::Value,
    ) -> Option<CorePluginFuture> {
        if handler_id != HANDLER_INVOKE {
            return None;
        }
        let ctx = self.context.clone();
        // Lock + compose synchronously so the future doesn't borrow
        // the registry (which is `!Send` across an .await on some
        // platforms via the Mutex guard). Failures here surface as
        // immediate `Err` futures.
        let composed = match self.compose_for_invoke(args) {
            Ok(c) => c,
            Err(e) => return Some(Box::pin(async move { Err(e) })),
        };
        let raw_args = args.clone();
        Some(Box::pin(async move {
            handle_invoke(ctx.as_ref(), composed, raw_args).await
        }))
    }

    fn wire_context(&mut self, ctx: Arc<KernelPluginContext>) {
        self.context = Some(ctx);
    }
}

impl SkillsCorePlugin {
    fn dispatch_list(&self) -> Result<serde_json::Value, PluginError> {
        let reg = self.registry.lock().map_err(poisoned)?;
        // BL-022 — augment each entry with `relpath` (forge-relative
        // path) so the in-app editor can call `com.nexus.storage::
        // write_file` / `delete_file` without an extra round trip.
        // Append-only on the wire — pre-BL-022 consumers ignore the
        // new field.
        let mut out = Vec::with_capacity(reg.len());
        for (path, skill) in reg.entries() {
            let mut value = serde_json::to_value(skill)
                .map_err(|e| exec_err(format!("list: serialize: {e}")))?;
            if let Some(rel) = relpath_for(&self.root, path) {
                value["relpath"] = serde_json::Value::String(rel);
            }
            out.push(value);
        }
        Ok(serde_json::Value::Array(out))
    }

    fn dispatch_get(&self, args: &serde_json::Value) -> Result<serde_json::Value, PluginError> {
        let a: GetSkillArgs = parse_args(args, "get")?;
        let reg = self.registry.lock().map_err(poisoned)?;
        match reg.get(&a.id) {
            Some(skill) => {
                let mut value = serde_json::to_value(skill)
                    .map_err(|e| exec_err(format!("get: serialize: {e}")))?;
                if let Some(path) = reg.path_for(&a.id) {
                    if let Some(rel) = relpath_for(&self.root, path) {
                        value["relpath"] = serde_json::Value::String(rel);
                    }
                }
                Ok(value)
            }
            None => Err(exec_err(format!("no skill with id '{}'", a.id))),
        }
    }

    fn dispatch_list_by_context(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        let a: ListByContextArgs = parse_args(args, "list_by_context")?;
        let reg = self.registry.lock().map_err(poisoned)?;
        let skills: Vec<_> = reg.by_context(&a.context).cloned().collect();
        to_value(&skills, "list_by_context")
    }

    fn dispatch_triggered_by(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        let a: TriggeredByArgs = parse_args(args, "triggered_by")?;
        let reg = self.registry.lock().map_err(poisoned)?;
        let skills: Vec<_> = reg.triggered_by(&a.text).cloned().collect();
        to_value(&skills, "triggered_by")
    }

    fn dispatch_render(&self, args: &serde_json::Value) -> Result<serde_json::Value, PluginError> {
        let a: RenderSkillArgs = parse_args(args, "render")?;
        let reg = self.registry.lock().map_err(poisoned)?;
        let skill = reg
            .get(&a.id)
            .ok_or_else(|| exec_err(format!("no skill with id '{}'", a.id)))?;
        let values: std::collections::HashMap<String, serde_norway::Value> = a
            .values
            .into_iter()
            .map(|(k, v)| {
                // Round-trip JSON → YAML so enum comparisons match the
                // skill's declared `values:` list shape.
                let y = serde_norway::to_value(&v).unwrap_or(serde_norway::Value::Null);
                (k, y)
            })
            .collect();
        let rendered =
            crate::render(skill, &values).map_err(|e| exec_err(format!("render: {e}")))?;
        Ok(serde_json::json!({
            "id": skill.meta.id,
            "name": skill.meta.name,
            "body": rendered,
        }))
    }

    /// BL-021 — resolve a skill's `depends_on` closure. Returns the
    /// ordered fragment list, a merged body string, and any non-fatal
    /// conflict warnings. Cycle / missing-dependency are surfaced as
    /// `ExecutionFailed` so the planner can fall back to the raw body.
    fn dispatch_compose(&self, args: &serde_json::Value) -> Result<serde_json::Value, PluginError> {
        let a: ComposeSkillArgs = parse_args(args, "compose")?;
        let reg = self.registry.lock().map_err(poisoned)?;
        match crate::compose::compose(&reg, &a.id) {
            Ok(composed) => to_value(&composed, "compose"),
            Err(err) => Err(exec_err(format!("compose: {err}"))),
        }
    }

    /// BL-054 Phase 3 — pre-async setup for `invoke`. Parses the args,
    /// resolves the skill's `depends_on` closure, and returns the
    /// merged body the agent should use as its system prompt plus
    /// the [`EffectiveRestrictions`] folded across every ancestor in
    /// that closure (gap-closing fix — capability gate drop). Lives
    /// on `&self` so the locked registry guard never crosses an
    /// `.await` (which would make the future `!Send`).
    fn compose_for_invoke(
        &self,
        args: &serde_json::Value,
    ) -> Result<ComposedInvocation, PluginError> {
        let parsed: InvokeSkillArgs = parse_args(args, "invoke")?;
        if parsed.skill_id.is_empty() {
            return Err(exec_err("invoke: skill_id must not be empty".to_string()));
        }
        let reg = self.registry.lock().map_err(poisoned)?;
        if reg.get(&parsed.skill_id).is_none() {
            return Err(exec_err(format!(
                "invoke: no skill with id '{}'",
                parsed.skill_id
            )));
        }
        let composed = crate::compose::compose(&reg, &parsed.skill_id)
            .map_err(|e| exec_err(format!("invoke: compose: {e}")))?;
        let order: Vec<String> = composed.fragments.iter().map(|f| f.id.clone()).collect();
        let restrictions = fold_restrictions(&reg, &order);
        Ok(ComposedInvocation {
            body: composed.merged_body,
            restrictions,
        })
    }

    fn dispatch_reload(&self) -> Result<serde_json::Value, PluginError> {
        let reloaded = SkillRegistry::load(&self.root).unwrap_or_else(|err| {
            tracing::warn!(
                path = %self.root.display(),
                err = %err,
                "com.nexus.skills reload: partial or full failure; old registry replaced with parsed subset"
            );
            match err {
                SkillRegistryError::PartialParseFailure { .. } => {
                    // `load` returns the error AFTER populating the
                    // registry with the successfully-parsed subset,
                    // but discards it. Re-load once more into an
                    // empty registry so we at least keep whatever
                    // parsed cleanly.
                    SkillRegistry::load(&self.root).unwrap_or_else(|_| SkillRegistry::empty())
                }
                SkillRegistryError::Io(_) => SkillRegistry::empty(),
            }
        });
        let len = reloaded.len();
        // Best-effort: refresh the on-disk REGISTRY.json index so a
        // subsequent cold-start `load_with_index` reflects the new
        // walk. Failures log and do not abort the reload.
        let index_path = self.root.join("REGISTRY.json");
        if let Err(err) = registry_index::write_index(&index_path, &self.root, &reloaded) {
            tracing::warn!(
                path = %index_path.display(),
                err = %err,
                "com.nexus.skills reload: failed to refresh REGISTRY.json"
            );
        }
        *self.registry.lock().map_err(poisoned)? = reloaded;
        Ok(serde_json::json!({ "loaded": len }))
    }
}

// ── Error / serde plumbing ──────────────────────────────────────────────────

/// Compute the forge-relative path for a skill's source file. The
/// registry holds absolute paths; the shell-side editor wants the
/// `.forge/skills/<sub>/<name>.skill.md` form that
/// `com.nexus.storage::write_file` expects. Returns `None` when the
/// skill lives outside the configured `root` (shouldn't happen at
/// runtime — defensive against future asymmetry between the load
/// and save paths). Forward slashes always — the storage handler
/// canonicalises to the platform separator on its end.
fn relpath_for(skills_root: &std::path::Path, abs: &std::path::Path) -> Option<String> {
    let from_skills = abs.strip_prefix(skills_root).ok()?;
    let mut parts: Vec<String> = Vec::new();
    parts.push(".forge".to_string());
    parts.push("skills".to_string());
    for component in from_skills.components() {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    Some(parts.join("/"))
}

nexus_plugins::define_dispatch_helpers!();

fn poisoned<T>(_e: std::sync::PoisonError<T>) -> PluginError {
    exec_err("skills registry mutex poisoned — prior handler panicked".to_string())
}

/// Gap-closing fix (capability gate drop) — most-restrictive-wins
/// fold of every ancestor skill's `restrictions` block across a
/// `depends_on` closure (including the invoked skill itself).
/// Threaded into the `com.nexus.agent::session_run` payload's
/// `restrictions` field and into an `auto_approve` downgrade so a
/// restricted skill can't silently retain full tool access and
/// blind approval.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EffectiveRestrictions {
    execute_code: Option<bool>,
    modify_files: Option<bool>,
    delete_content: Option<bool>,
    /// Intersection of every non-empty ancestor `allowed_tools` list.
    /// Empty means no ancestor declared one (unconstrained), matching
    /// [`crate::SkillRestrictions::allowed_tools`]'s own "empty means
    /// unconstrained" semantics.
    allowed_tools: Vec<String>,
}

impl EffectiveRestrictions {
    /// `true` once at least one ancestor turned a lever off. Session
    /// invocation downgrades `auto_approve` whenever this holds.
    fn is_restrictive(&self) -> bool {
        self.execute_code == Some(false)
            || self.modify_files == Some(false)
            || self.delete_content == Some(false)
    }

    /// `None` when every field is at its unrestricted default — the
    /// `session_run` payload omits the `restrictions` key entirely
    /// rather than sending a no-op object.
    fn to_json(&self) -> Option<serde_json::Value> {
        if self.execute_code.is_none()
            && self.modify_files.is_none()
            && self.delete_content.is_none()
            && self.allowed_tools.is_empty()
        {
            return None;
        }
        Some(serde_json::json!({
            "execute_code": self.execute_code,
            "modify_files": self.modify_files,
            "delete_content": self.delete_content,
            "allowed_tools": self.allowed_tools,
        }))
    }
}

/// Most-restrictive-wins merge of a single boolean lever across two
/// ancestors: an explicit `false` anywhere in the chain always wins
/// (over `true` or unset); an explicit `true` beats unset; unset
/// stays unset when neither ancestor declared the lever.
fn fold_lever(acc: Option<bool>, next: Option<bool>) -> Option<bool> {
    match (acc, next) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), _) | (_, Some(true)) => Some(true),
        (None, None) => None,
    }
}

/// Fold every skill in `order` (the topologically-sorted
/// `depends_on` closure `compose` already resolved — dependencies
/// first, root last) into one [`EffectiveRestrictions`].
fn fold_restrictions(reg: &SkillRegistry, order: &[String]) -> EffectiveRestrictions {
    let mut out = EffectiveRestrictions::default();
    let mut allow_lists: Vec<Vec<String>> = Vec::new();
    for id in order {
        let Some(skill) = reg.get(id) else { continue };
        let Some(r) = skill.meta.restrictions.as_ref() else {
            continue;
        };
        out.execute_code = fold_lever(out.execute_code, r.execute_code);
        out.modify_files = fold_lever(out.modify_files, r.modify_files);
        out.delete_content = fold_lever(out.delete_content, r.delete_content);
        if !r.allowed_tools.is_empty() {
            allow_lists.push(r.allowed_tools.clone());
        }
    }
    if let Some(first) = allow_lists.first().cloned() {
        out.allowed_tools = allow_lists.into_iter().fold(first, |acc, list| {
            let set: std::collections::HashSet<&String> = list.iter().collect();
            acc.into_iter().filter(|t| set.contains(t)).collect()
        });
    }
    out
}

/// Result of [`SkillsCorePlugin::compose_for_invoke`] — the merged
/// system-prompt body plus the folded restriction constraint the
/// async `invoke` handler must enforce on the dispatched session.
struct ComposedInvocation {
    body: String,
    restrictions: EffectiveRestrictions,
}

/// Pure builder for the `com.nexus.agent::session_run` payload —
/// split out from [`handle_invoke`] so the capability-downgrade
/// logic is unit-testable without a live `KernelPluginContext`/IPC.
/// A restrictive fold forces `auto_approve: false` (interactive
/// approval gate instead of blind auto-approval) and always attaches
/// the folded `restrictions` object so `session_run`'s tool-policy
/// gate denies unreachable capabilities/tools even if a future
/// caller re-enables `auto_approve` upstream of this handler.
fn build_invoke_payload(
    goal: &str,
    archetype: &str,
    composed_body: &str,
    restrictions: &EffectiveRestrictions,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "goal": goal,
        "archetype": archetype,
        "system": composed_body,
        "auto_approve": !restrictions.is_restrictive(),
    });
    if let Some(r) = restrictions.to_json() {
        payload["restrictions"] = r;
    }
    payload
}

/// BL-054 Phase 3 — async handler for `com.nexus.skills::invoke`.
/// Composes the skill body (precomputed by `compose_for_invoke` so
/// the registry lock doesn't cross the `.await`) and dispatches
/// `com.nexus.agent::session_run` with `goal = input`,
/// `system = composed body`, `archetype = arg ?? "general"`. Gap-
/// closing fix — `auto_approve` and an explicit `restrictions` object
/// are derived from the skill's folded `depends_on` restrictions
/// (see [`build_invoke_payload`]) instead of always sending
/// `auto_approve: true` with no constraint. Returns the agent's reply
/// verbatim — the caller decides how to render the observation.
async fn handle_invoke(
    ctx: Option<&Arc<KernelPluginContext>>,
    composed: ComposedInvocation,
    args: serde_json::Value,
) -> Result<serde_json::Value, PluginError> {
    let parsed: InvokeSkillArgs =
        serde_json::from_value(args).map_err(|e| exec_err(format!("invoke: invalid args: {e}")))?;
    let ctx = ctx.ok_or_else(|| {
        exec_err(
            "invoke: no kernel context wired (bootstrap did not call wire_context)".to_string(),
        )
    })?;
    let archetype = parsed
        .archetype
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_ARCHETYPE)
        .to_string();
    let payload = build_invoke_payload(
        &parsed.input,
        &archetype,
        &composed.body,
        &composed.restrictions,
    );
    ctx.ipc_call(
        "com.nexus.agent",
        "session_run",
        payload,
        INVOKE_AGENT_TIMEOUT,
    )
    .await
    .map_err(|e| exec_err(format!("invoke: agent::session_run failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const SKILL_A: &str = r#"---
name: A
id: skill-a
description: first
version: 1.0.0
author: me
created: 2026-04-01
tags: [alpha]
applicable_contexts: [ai-chat]
triggers: ["alpha mode"]
---
body A
"#;

    fn write_skill(dir: &std::path::Path, filename: &str, contents: &str) {
        std::fs::write(dir.join(filename), contents).unwrap();
    }

    /// Drive a single future to completion on a fresh tokio current-
    /// thread runtime — keeps the unit tests free of an extra
    /// `futures` workspace dep just for `block_on`.
    fn block_on_test<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    #[test]
    fn list_round_trips_through_dispatch() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "a.skill.md", SKILL_A);
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let v = plugin
            .dispatch(HANDLER_LIST, &serde_json::json!({}))
            .unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "skill-a");
    }

    #[test]
    fn get_returns_error_for_unknown_id() {
        let tmp = TempDir::new().unwrap();
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let err = plugin
            .dispatch(HANDLER_GET, &serde_json::json!({ "id": "missing" }))
            .unwrap_err();
        match err {
            PluginError::ExecutionFailed { reason, .. } => {
                assert!(reason.contains("no skill"));
            }
            _ => panic!("unexpected error"),
        }
    }

    #[test]
    fn list_by_context_filters_correctly() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "a.skill.md", SKILL_A);
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let v = plugin
            .dispatch(
                HANDLER_LIST_BY_CONTEXT,
                &serde_json::json!({ "context": "editor" }),
            )
            .unwrap();
        assert_eq!(v.as_array().unwrap().len(), 0);
        let v = plugin
            .dispatch(
                HANDLER_LIST_BY_CONTEXT,
                &serde_json::json!({ "context": "ai-chat" }),
            )
            .unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
    }

    #[test]
    fn triggered_by_matches_case_insensitively() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "a.skill.md", SKILL_A);
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let v = plugin
            .dispatch(
                HANDLER_TRIGGERED_BY,
                &serde_json::json!({ "text": "please enter ALPHA MODE" }),
            )
            .unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
    }

    #[test]
    fn render_substitutes_declared_parameters() {
        const SKILL_WITH_PARAM: &str = r"---
name: P
id: skill-p
description: d
version: 1.0.0
author: me
created: 2026-04-18
parameters:
  - name: tone
    type: string
    default: friendly
---
Write in a {{ tone }} style.
";
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "p.skill.md", SKILL_WITH_PARAM);
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let v = plugin
            .dispatch(
                HANDLER_RENDER,
                &serde_json::json!({ "id": "skill-p", "values": { "tone": "formal" } }),
            )
            .unwrap();
        assert_eq!(v["body"], "Write in a formal style.\n");

        let v = plugin
            .dispatch(HANDLER_RENDER, &serde_json::json!({ "id": "skill-p" }))
            .unwrap();
        assert_eq!(v["body"], "Write in a friendly style.\n");
    }

    #[test]
    fn render_errors_on_unknown_skill() {
        let tmp = TempDir::new().unwrap();
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let err = plugin
            .dispatch(HANDLER_RENDER, &serde_json::json!({ "id": "missing" }))
            .unwrap_err();
        match err {
            PluginError::ExecutionFailed { reason, .. } => {
                assert!(reason.contains("no skill"));
            }
            _ => panic!("unexpected error"),
        }
    }

    #[test]
    fn open_writes_registry_json_after_load() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "a.skill.md", SKILL_A);
        let _plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());

        let index_path = tmp.path().join("REGISTRY.json");
        assert!(index_path.is_file(), "open must persist REGISTRY.json");
        let parsed = crate::registry_index::read_index(&index_path).unwrap();
        assert_eq!(parsed.skills.len(), 1);
        assert_eq!(parsed.skills[0].id, "skill-a");
    }

    #[test]
    fn reload_handler_rewrites_index() {
        const SKILL_B: &str = r"---
name: B
id: skill-b
description: second
version: 1.0.0
author: me
created: 2026-04-02
---
body B
";
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "a.skill.md", SKILL_A);
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());

        // After open, the index lists exactly skill-a.
        let index_path = tmp.path().join("REGISTRY.json");
        let initial = crate::registry_index::read_index(&index_path).unwrap();
        assert_eq!(initial.skills.len(), 1);

        // Add a second skill on disk and trigger a reload.
        write_skill(tmp.path(), "b.skill.md", SKILL_B);
        let v = plugin
            .dispatch(HANDLER_RELOAD, &serde_json::json!({}))
            .unwrap();
        assert_eq!(v["loaded"], 2);

        let after = crate::registry_index::read_index(&index_path).unwrap();
        assert_eq!(after.skills.len(), 2);
        let ids: Vec<&str> = after.skills.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"skill-a"));
        assert!(ids.contains(&"skill-b"));
    }

    #[test]
    fn reload_picks_up_new_files() {
        let tmp = TempDir::new().unwrap();
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        assert_eq!(
            plugin
                .dispatch(HANDLER_LIST, &serde_json::json!({}))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            0
        );
        write_skill(tmp.path(), "a.skill.md", SKILL_A);
        let v = plugin
            .dispatch(HANDLER_RELOAD, &serde_json::json!({}))
            .unwrap();
        assert_eq!(v["loaded"], 1);
    }

    // ── BL-054 Phase 3 — invoke routing ─────────────────────────────────

    #[test]
    fn invoke_sync_dispatch_directs_caller_to_async() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "a.skill.md", SKILL_A);
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let err = plugin
            .dispatch(
                HANDLER_INVOKE,
                &serde_json::json!({ "skill_id": "skill-a", "input": "go" }),
            )
            .unwrap_err();
        match err {
            PluginError::HandlerIsAsyncOnly { handler_id } => {
                assert_eq!(handler_id, HANDLER_INVOKE);
            }
            _ => panic!("unexpected error"),
        }
    }

    #[test]
    fn invoke_async_without_context_returns_clear_error() {
        // The bootstrap normally calls `wire_context` after registering
        // the plugin. Unit-test plugins skip that, so the future
        // should surface a useful message instead of panicking.
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "a.skill.md", SKILL_A);
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let fut = plugin
            .dispatch_async(
                HANDLER_INVOKE,
                &serde_json::json!({ "skill_id": "skill-a", "input": "go" }),
            )
            .expect("dispatch_async returned None for HANDLER_INVOKE");
        let err = block_on_test(fut).unwrap_err();
        match err {
            PluginError::ExecutionFailed { reason, .. } => {
                assert!(reason.contains("no kernel context"), "got: {reason}");
            }
            _ => panic!("unexpected error"),
        }
    }

    #[test]
    fn invoke_async_unknown_skill_id_short_circuits() {
        let tmp = TempDir::new().unwrap();
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let fut = plugin
            .dispatch_async(
                HANDLER_INVOKE,
                &serde_json::json!({ "skill_id": "missing", "input": "go" }),
            )
            .expect("dispatch_async returned None for HANDLER_INVOKE");
        let err = block_on_test(fut).unwrap_err();
        match err {
            PluginError::ExecutionFailed { reason, .. } => {
                assert!(reason.contains("no skill with id"), "got: {reason}");
            }
            _ => panic!("unexpected error"),
        }
    }

    #[test]
    fn invoke_async_returns_none_for_unrelated_handler() {
        let tmp = TempDir::new().unwrap();
        let mut plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        assert!(plugin
            .dispatch_async(HANDLER_LIST, &serde_json::json!({}))
            .is_none());
    }

    // ── gap-closing fix — capability gate drop (restrictions enforcement) ──

    const SKILL_RESTRICTED: &str = r"---
name: Restricted
id: skill-restricted
description: may not run code or delete anything
version: 1.0.0
author: me
created: 2026-04-03
restrictions:
  execute_code: false
  allowed_tools: []
---
body restricted
";

    const SKILL_CHILD_OF_RESTRICTED: &str = r"---
name: Child
id: skill-child
description: layers on the restricted base
version: 1.0.0
author: me
created: 2026-04-04
depends_on: [skill-restricted]
---
body child
";

    #[test]
    fn build_invoke_payload_downgrades_auto_approve_when_restrictive() {
        let restrictions = EffectiveRestrictions {
            execute_code: Some(false),
            modify_files: None,
            delete_content: None,
            allowed_tools: Vec::new(),
        };
        let payload = build_invoke_payload("do the thing", "general", "system body", &restrictions);
        assert_eq!(payload["auto_approve"], false);
        assert_eq!(payload["restrictions"]["execute_code"], false);
        assert_eq!(
            payload["restrictions"]["allowed_tools"],
            serde_json::json!([])
        );
    }

    #[test]
    fn build_invoke_payload_keeps_auto_approve_true_when_unrestricted() {
        let restrictions = EffectiveRestrictions::default();
        let payload = build_invoke_payload("do the thing", "general", "system body", &restrictions);
        assert_eq!(payload["auto_approve"], true);
        assert!(
            payload.get("restrictions").is_none(),
            "an unrestricted fold must not attach a no-op restrictions object"
        );
    }

    #[test]
    fn compose_for_invoke_folds_restrictions_for_a_single_restricted_skill() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "restricted.skill.md", SKILL_RESTRICTED);
        let plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let composed = plugin
            .compose_for_invoke(&serde_json::json!({
                "skill_id": "skill-restricted",
                "input": "go",
            }))
            .unwrap();
        assert_eq!(composed.restrictions.execute_code, Some(false));
        assert!(composed.restrictions.allowed_tools.is_empty());
        assert!(composed.restrictions.is_restrictive());

        // Pre-fix, `handle_invoke` always sent `auto_approve: true`
        // with no `restrictions` key regardless of this skill's
        // frontmatter — a `session_run` call with full capabilities
        // and blind approval. Post-fix the payload is constrained.
        let payload = build_invoke_payload("go", "general", &composed.body, &composed.restrictions);
        assert_eq!(payload["auto_approve"], false);
        assert_eq!(payload["restrictions"]["execute_code"], false);
    }

    #[test]
    fn compose_for_invoke_folds_restrictions_across_depends_on_chain() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "restricted.skill.md", SKILL_RESTRICTED);
        write_skill(tmp.path(), "child.skill.md", SKILL_CHILD_OF_RESTRICTED);
        let plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        // Invoking the *child* (which does not itself declare
        // restrictions) must still inherit the restrictive lever from
        // its `skill-restricted` ancestor — most-restrictive-wins.
        let composed = plugin
            .compose_for_invoke(&serde_json::json!({
                "skill_id": "skill-child",
                "input": "go",
            }))
            .unwrap();
        assert_eq!(composed.restrictions.execute_code, Some(false));
        assert!(composed.restrictions.is_restrictive());
    }

    #[test]
    fn fold_restrictions_intersects_allowed_tools_across_ancestors() {
        const SKILL_ALLOW_AB: &str = r"---
name: AllowAB
id: skill-allow-ab
description: d
version: 1.0.0
author: me
created: 2026-04-05
restrictions:
  allowed_tools: [read_file, write_file]
---
base
";
        const SKILL_ALLOW_B_ONLY: &str = r"---
name: AllowBOnly
id: skill-allow-b
description: d
version: 1.0.0
author: me
created: 2026-04-06
depends_on: [skill-allow-ab]
restrictions:
  allowed_tools: [write_file]
---
child
";
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "a.skill.md", SKILL_ALLOW_AB);
        write_skill(tmp.path(), "b.skill.md", SKILL_ALLOW_B_ONLY);
        let plugin = SkillsCorePlugin::open(tmp.path().to_path_buf());
        let composed = plugin
            .compose_for_invoke(&serde_json::json!({
                "skill_id": "skill-allow-b",
                "input": "go",
            }))
            .unwrap();
        assert_eq!(composed.restrictions.allowed_tools, vec!["write_file"]);
    }
}
