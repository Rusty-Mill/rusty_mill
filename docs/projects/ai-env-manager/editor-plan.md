# In-app file editor: design plan

Status: plan only, no code. Answers the "how" of D37 (Nano, 2026-11-14: "I'd like it in"; in-app
editing is in GUI v1, widening D34). Order stays D33/D35 (CLI, GUI, MCP). Scope is settled by D38 (Q15);
choices below marked `[default]` are implementation detail and get logged through
`ai-env-manager-expert` when a slice starts.

## What "it" is

Nano's note (`mem_3e1c2433807747f4a6a94a1fc9e72790`): "I need this to be a place where I can also
modify and edit the files", for the cataloged files. The GUI today (D34) lists entries and
enables or disables them. The editor adds: open an entry's file, change its text, save.

## Which files are editable

| File | Editable? | Why |
|---|---|---|
| Entry source file(s) inside the entry's source folder (where `aienv` links from, D5) | Yes, with confirm | This is the file Nano means. The folder is his, not ours: we never wrote a marker in it |
| A marked copy at a tool's target path (copy fallback, D6) | Yes, with confirm | It carries our marker. Editing it moves the entry to the existing "edited" state (`Observed::ManagedModified`), which already blocks disable or replace so the edit is never lost |
| A symlink or junction target | Edit goes to the source file it points at | There is one file; the link is not a second copy |
| Anything at a target path without our marker (foreign, `Unmanaged`) | No. Shown read-only at most | D6: never replace or remove, and we do not rewrite what we cannot prove we own |
| Any path outside a catalog entry's folder | No | D8, D32: no file passthrough |

`[default]` Only files inside a catalog entry's own folder are reachable; the editor never takes a
path from the webview, only an entry id plus a path relative to that entry.
Stated (D38, Q15a): skill files in the catalog today, widening as the catalog does; plugins, agents,
mods and key md files (CLAUDE.md, AGENTS.md) join when the catalog holds them. Today the catalog
holds skills only (D29), so v1 editing covers skills.

## How it fits plan-first and consent (D6, D8)

Same shape as enable/disable: nothing is written until the user confirms.

1. `edit_preview`: the backend validates the target, reads the file, and returns an `EditPlan`
   (path, ownership class, a line diff of old versus new text, a warning when the file is a
   marked copy or is shared by several tools through links). It writes nothing.
2. The GUI shows the diff and a Confirm button, the GUI form of `--yes` (D32).
3. `edit_apply`: writes only if the plan is still valid (see "file changed on disk").

Rules the backend enforces, not the webview:
- D6 holds: an edit never creates, replaces or removes a path; it overwrites the content of an
  existing regular file that is a source file or a marked copy. A marker file is never editable.
- Unmanaged paths are refused with a reason, exactly like a refused link (D34 "refusals shown").
- Write is atomic: temp file in the same folder, then rename; the original mode is kept.
- No telemetry, no network, no write outside the entry folder (D8). Every edit is reversible:
  the previous text is kept as one in-memory undo step in the session and offered as "Revert",
  which is itself a previewed edit. `[default]` no on-disk backup files in v1.

## Backend commands (the GUI holds no logic, D32)

All logic goes in `ai_env_app` (shared with the CLI, so `aienv edit` can follow later at no extra
design cost); `gui/src/lib.rs` only forwards typed commands.

| Command | Input | Output |
|---|---|---|
| `list_files` | entry id | files inside the entry, each with ownership class (source, marked copy, foreign, link) and editable flag |
| `read_file` | entry id, relative path | text, size, content hash, ownership class |
| `edit_preview` | entry id, relative path, new text, hash read | `EditPlan` (diff, class, warnings) or a refusal with its reason |
| `edit_apply` | the `EditPlan` | applied, or refused (hash mismatch, class changed) |

Core (`ai_env_core`) gains the pure parts: `EditPlan`, the class decision, size and type limits.
`ai_env_fs` gains the one disk write and the hash read. `[default]` content hash is the existing
`tree_hash`-style hash applied to a single file, so no new dependency (D12). `[default]` the diff
is computed in `ai_env_app` (line-based, std only), not in JavaScript.

## Tauri shell and security constraints (D31, D32)

- No new plugin, no `fs`, `shell`, `http` or `dialog` plugin; capabilities stay `core:default`
  with the four typed commands added. The webview never receives an absolute path to write.
- CSP stays `default-src 'none'; script-src 'self'; style-src 'self'; connect-src ipc:
  http://ipc.localhost`. A plain `<textarea>` editor needs no CSP relaxation. A code editor
  library (CodeMirror, Monaco) is a new JS dependency and workers/inline styles: ask-first (D12)
  and out of v1. `[default]` textarea with a monospace font, line numbers optional.
- Confirm is enforced only in the page: any script in the webview could call `apply_edit`
  without `preview_edit`. That is fine while the page loads only local assets under this CSP.
  Do not loosen the CSP assuming the backend gates consent.
- File content is rendered as text only (`textContent`), never as HTML; Markdown preview, if added
  later, goes through an escaping renderer with no inline script and no remote images.
- Path handling in the backend: reject absolute paths, `..`, and any component that is a symlink
  leaving the entry folder (canonicalize, then check the prefix). Refuse non-UTF-8 and binary
  files, and files over a size cap. `[default]` cap 1 MiB.
- Errors shown to the user are the backend's reason strings; the backend does not log file text.

## Edge cases

- **File changed on disk between read and save.** `edit_apply` re-hashes; on mismatch it refuses
  and the GUI offers to reload, showing both versions. Never overwrite silently.
- **Edited managed copy.** A save to a marked copy is allowed after confirm. Afterwards the entry
  reads "edited" (`ManagedModified`); disable and re-enable refuse until the user resolves it, as
  today. `[default]` v1 does not offer "push the edit back to the source" or "discard the edit";
  that is a follow-up and would be its own previewed plan.
- **Shared file.** If several tools link to one source, the preview says so ("this file is used by
  Claude Code and Codex"), because one save changes all of them.
- **Edit while the entry is being enabled or disabled.** The GUI blocks the other action while an
  edit plan is open, and the backend re-checks the class at apply time.
- **Foreign or refused target.** Shown read-only with the refusal reason; no Save.
- **Permissions, read-only file, disk full.** Reported with the OS reason; the original is
  untouched because the write is temp-then-rename.
- **Line endings and trailing newline.** Preserved as found. `[default]`.
- **Unsaved changes on close or entry switch.** The GUI prompts. `[default]`.

## Slice order

Each slice is one change in flight (D16), docs and tests first.

1. Core: `EditPlan`, ownership class for a file, path validation, limits, tests (no disk write).
2. Fs and app: hash read, atomic write, `list_files`, `read_file`, `edit_preview`, `edit_apply`,
   with tests for each refusal (foreign, marker file, symlink escape, changed-on-disk, binary).
3. GUI shell: register the four commands, capability entry, no CSP change.
4. GUI view: file list, read-only viewer, then textarea editor with diff preview and Confirm.
5. Edge polish: unsaved-change prompt, shared-file warning, revert step, "edited" badge wiring.
6. Optional later: `aienv edit` in the CLI, source-side "push edit back", code-editor widget
   (D12 ask), plugins, agents and key md files once the catalog holds them.

## Scope (stated, D38, was Q15)

Nano took the recommended scope on all three points (D38, 2026-11-14).

1. **Which files count (Q15a).** Skill files in the catalog today, widening as the catalog does.
2. **What "edit" covers (Q15b).** Text edits to existing files only; no create, rename or delete,
   and no new entries.
3. **Editing a source folder outside our own folders (Q15c).** Allowed with the per-save diff
   preview plus Confirm (D8). No one-time allow-editing setting.
