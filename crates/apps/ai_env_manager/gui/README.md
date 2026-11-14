# ai-env-gui

Ai Env Manager's desktop shell (Tauri 2). A view over `ai_env_app`: it lists skills against tools with their state and enables or disables them behind a plan preview. It holds no logic of its own (pack D32, `docs/projects/ai-env-manager/handoff/DECISIONS.md`).

## Run

```sh
cd crates/apps/ai_env_manager/gui
cargo run -- --skills ~/skill_pack/skills            # every tool in the catalog
cargo run -- --skills DIR --skills DIR2 --tool codex # fixed at launch
```

`--skills DIR` is required and repeatable, like `aienv`; `--tool ID` is optional. The page cannot change them. On Linux the Tauri build needs WebKitGTK (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`).

## Shape

- `src/lib.rs`: three typed commands (`list`, `plan_change`, `apply_change`). `apply_change` plans again and never trusts a plan from the page.
- `src/view.rs`: the serializable views that cross the boundary (pure mapping).
- `ui/`: plain HTML, CSS and JS. No npm toolchain, strict CSP, text set with `textContent` only.
- `capabilities/default.json`: `core:default` only; no fs, shell, http or dialog plugin.

## Test

```sh
cargo test --locked && cargo clippy --locked --all-targets -- -D warnings
```

This is a standalone workspace (excluded from the root one, like `rusty_key`'s desktop shell). The `ai-env-gui` CI job runs the two commands above when this folder or an `ai_env_*` crate changes.
