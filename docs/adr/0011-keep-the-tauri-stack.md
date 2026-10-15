# ADR-0011: Keep the Tauri stack external

Status: Accepted
Date: 2026-10-15

## Context

ADR-0002 lists "Tauri desktop shells" as Tier A (adapter/application): an
external dependency behind a first-party boundary that is not expected to
reach zero. It does not record a keep-or-replace decision for the Tauri
crates themselves. Issue #704 asks for that decision, so the "Sovereign
foundation" goal in `ARCHITECTURE.md` has a recorded answer for the desktop
shells rather than an open question.

Counts are direct dependencies in `Cargo.toml` files under `crates/` plus the
root `Cargo.toml`, with the frozen Nexus crates excluded from the verdict,
checked 2026-10-15. This is not a vetted audit.

| Crate | Where it is used | Ships in a release artifact |
| --- | --- | --- |
| `tauri` | `[dependencies]` of three desktop shells: `inventory-tauri` (workspace member), `sessionmgr-desktop` (workspace member), `rusty-keys-desktop` (own `[workspace]`, listed under root `exclude`); `[dev-dependencies]` of `inventory-tauri` and `rusty-keys-desktop` with the `test` feature | yes, as the shell runtime |
| `tauri-build` | `[build-dependencies]` of the same three shells (two through the root `[workspace.dependencies]` entry) | no, build script only |
| `tauri-plugin-global-shortcut` | `inventory-tauri` | yes |
| `tauri-plugin-clipboard-manager` | `inventory-tauri` | yes |
| `tauri-plugin-dialog` | root `[workspace.dependencies]` entry and the frozen `nexus-shell`; no unfrozen member uses it, and it sits in the `cargo-shear` ignore list | no unfrozen crate |

`nexus-shell` also uses `tauri-plugin-fs`, `-deep-link`, `-window-state` and
`-notification`. Nexus is frozen (`crates/apps/nexus/FROZEN.md`), so they are
outside this verdict and are decided if Nexus is unfrozen.

An in-house replacement exists for none of these. The nearest first-party
pieces are `rusty_gui` (a `no_std` windowing, event-loop and clipboard
crate), `rusty_font` and `rusty_gpu`. None is a web view, and the three
shells are written as a web front end (`ui/`, `index.html`, `vite.config.ts`)
talking to Rust commands.

## Decision

Keep `tauri`, `tauri-build`, `tauri-plugin-global-shortcut` and
`tauri-plugin-clipboard-manager` external. No replacement work is scheduled.

| Crate | Verdict | Reason |
| --- | --- | --- |
| `tauri` | Keep | It binds the operating system's web view (WebView2, WKWebView, WebKitGTK), the window and tray layer, and the IPC bridge. Replacing it means writing a cross-platform web-view host, which is a large, security-sensitive body of work; the web view engine itself would stay an operating-system component either way. |
| `tauri-build` | Keep | Build-time only. It is tied to `tauri` and goes when `tauri` goes. |
| `tauri-plugin-global-shortcut` | Keep | Tied to `tauri`'s plugin API. `inventory-tauri` needs a system-wide hotkey, which is per-platform FFI. |
| `tauri-plugin-clipboard-manager` | Keep | Tied to `tauri`'s plugin API. Clipboard access is per-platform FFI. |
| `tauri-plugin-dialog` | Not decided here | Only the frozen Nexus shell uses it. The root `[workspace.dependencies]` entry is a candidate to remove, as follow-up work, not part of this ADR. |

Under ADR-0002 these shells stay Tier A. The tier is a claim about their own
manifests; they may keep depending on Tier S crates. A desktop shell must not
be described as dependency-free.

Real `tokio` comes with `tauri`. `sessionmgr-desktop` already works around
this by keeping its socket client on blocking `std` I/O instead of mixing in
`rusty_tokio`; new shells should follow that rule and not run `rusty_tokio`
tasks inside a `tauri` command handler.

## Alternatives considered

**Replace `tauri` with the first-party GUI stack (`rusty_gui`, `rusty_font`,
`rusty_gpu`).** Rejected for now. It is a rewrite of three apps' front ends,
not a dependency swap: the shells are web UIs, and the first-party stack has
no web view, no tray or menu-bar layer, no global shortcut and no installer
bundler (the stack is a window and event layer, not an HTML host). A native
rewrite would drop the platform web view from the trust base, but the shells
would still rely on the operating system's window system.

**Write a first-party web-view host over the operating system's web view.**
Rejected for now. It would replace `tauri` and `wry` with code that does the
same per-platform FFI. The benefit is ownership, but the cost is a
cross-platform windowing, IPC and update surface that is easy to get wrong.

**Drop the desktop shells and keep CLI and TUI only.** Not decided here. That
is a product decision about three apps, not a dependency decision.

## Revisit when

Reopen this decision if any of these happens:

- A release product needs a desktop build with no registry crates, or Tier S
  is required for a shell.
- `rusty_gui` gains a web-view host or an HTML renderer, or a shell is
  rewritten as a native UI and no longer needs a web view.
- A security advisory or maintenance halt hits `tauri` or `wry` that is not
  fixed within one release cycle.
- Only one shell still uses `tauri`, or all remaining shells are retired,
  which makes dropping it cheap.
- The `tauri` major version changes in a way that forces a rewrite of the
  shells anyway.
- Nexus is unfrozen, which brings its five other plugins into scope.
