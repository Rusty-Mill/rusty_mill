# ai-env-gui

Ai Env Manager's desktop shell (Tauri 2). A view over `ai_env_app`: it lists skills against tools with their state and enables or disables them behind a plan preview. It holds no logic of its own (pack D32, `docs/projects/ai-env-manager/handoff/DECISIONS.md`).

## Run

```sh
cd crates/apps/ai_env_manager/gui
cargo run -- --skills ~/skill_pack/skills            # every tool in the catalog
cargo run -- --skills DIR --skills DIR2 --tool codex # more folders, one tool
cargo run                                            # then press "Choose folder"
```

`--skills DIR` and `--tool ID` are optional and repeatable. Every skills folder is searched at any depth, like `aienv`. "Choose folder" opens the native folder dialog from the backend and adds the folder for the session (not saved); the page never sends a path. The tools are fixed at launch. On Linux the Tauri build needs WebKitGTK (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`).

## Shape

- `src/lib.rs`: ten typed commands: `list`, `plan_change`, `apply_change`, the editor's `files`, `file`, `preview_edit`, `apply_edit`, and `preview_push`, `apply_push` (copy a tool copy's edits back to the source), and `choose_folder` (native dialog). `apply_change` plans again and never trusts a plan from the page; `apply_edit` re-checks every rule from the same inputs as `preview_edit`. The editor takes an entry id, an optional tool id and a path relative to the entry, never an absolute path.
- `src/view.rs`: the serializable views that cross the boundary (pure mapping).
- `ui/`: plain HTML, CSS and JS, including the editor dialog (file list, viewer with line numbers and Markdown colouring in `code.js`, textarea, diff, Confirm, unsaved-change prompt). `tests/ui/*.cjs` are standalone Playwright checks against a mocked backend; CI does not run them. No npm toolchain, strict CSP, text set with `textContent` only.
- `capabilities/default.json`: `core:default` only; no fs, shell or http plugin. The dialog plugin is initialised for the backend only; the page has no dialog permission.

## Test

```sh
cargo test --locked && cargo clippy --locked --all-targets -- -D warnings
```

This is a standalone workspace (excluded from the root one, like `rusty_key`'s desktop shell). The `ai-env-gui` CI job runs the two commands above when this folder or an `ai_env_*` crate changes.
