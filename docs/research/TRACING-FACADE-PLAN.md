# Plan: `tracing` facade (sovereignty list item 12)

Status: **option B approved by the owner (2026-10-19)**. Step 1 (the `rusty_log` crate and the baseline check) is done. Step 2 is in progress: `rusty-mcp` and the eight agent gateway crates are migrated; the other 22 baseline crates are not. Option C is not approved and is not planned.
Context: `RUSTY-MCP-SOVEREIGNTY.md` item 12 and D3; ADR-0002.

## What was measured (2026-10-18, `cargo metadata --all-features`, main at `25dd9517`)

- **30 workspace crates** depend on `tracing` (`rusty_tokio` also as a dev dependency; `rusty_lsp` and `rusty_tokio` optionally). The "74 crates" in the sovereignty list does not reproduce; it likely counted the excluded Nexus workspace or the lockfile.
- **9 crates** depend on `tracing-subscriber` (all with `env-filter` and `fmt`): seven binaries or services, `rusty-mcp`, and `rusty-acp` as a dev dependency only.
- About **380 `tracing` macro call sites** in the workspace (gateway 97, protocol 92, provider 73, tailscale 57, croc 40, the rest under 10 each). Nexus has another 666 but is excluded from the workspace and frozen.
- Beyond `info!` and friends the workspace uses: spans (`info_span!` and friends, 11 files), `Instrument` (2 files), `Span::current` (8 files), custom `Layer`s and `Subscriber`s (10 files, among them `rusty-mcp`'s `SpanLayer` for OTLP spans), and `EnvFilter` / `fmt` setup (about 30 files).
- **Third-party crates in the build graph also depend on `tracing`**: `axum`, `tower`, `tower-http`, `hyper-util`, `h2`, `tonic`, `sqlx-*`, the `aws-*` family, `boringtun`, `winit`, `zbus`. They emit their events through `tracing`'s global dispatcher.

## What that means

1. **`tracing` cannot leave the build graph** while those crates stay. An in-house facade removes our *direct* dependency, not the crate.
2. **An in-house subscriber would go deaf to them.** If our binaries stop installing a `tracing` subscriber, the `h2`, `sqlx`, `aws` and `axum` events vanish, which are the diagnostics one wants when a connection misbehaves. Keeping them means keeping a `tracing` subscriber, which defeats the point.
3. So the sovereignty gain of owning the *implementation* is small until the gateway leaves axum and the heavy third-party stack is gone (a separate, much larger effort; see `RUSTY-AXUM-SCOPE.md`).

## Options

| | What | Cost | Gain |
|---|---|---|---|
| **A. Keep** | Leave `tracing` as a named Tier A exception (D3 as it stands) | none | none; 30 manifests keep a direct external dependency |
| **B. Chokepoint** | New `rusty_log` crate that re-exports the `tracing` macros, `Span`, `Level`, `Instrument`, and the subscriber pieces we use (`Layer`, `registry`, `EnvFilter`, `fmt`) behind features. Migrate the 30 manifests and the `use tracing::` / `tracing_subscriber::` paths to it. Add a policy check that `tracing*` appears only in `rusty_log` | mechanical, about 5 PRs by area (protocol and gateway, provider, tailscale and croc, homelab and the rest), no behaviour change | one place to swap or pin; 30 direct external dependencies become one; the allow-list in `check_workspace_deps.py` shrinks to a single named exception |
| **C. Own it** | Implement events, spans, filtering, a fmt layer and `Layer` composition in-house, replacing `tracing` for our code | large (span ids and lineage, thread-local and global dispatch, async `Instrument`, env filter, JSON output, porting `SpanLayer`) | removes our direct use, but see points 1 and 2: third-party events are lost or a `tracing` bridge stays |

## Recommendation

**Do B; do not start C.** B gets the real policy benefit (a single audited entry point, a mechanical lint) for a few mechanical PRs and zero behaviour change, and it is what makes C cheap later if the third-party stack ever goes. C costs weeks and, by points 1 and 2, buys nothing visible while `axum`, `h2`, `sqlx` and `aws-*` are in the graph.

If B is approved, the order is:

1. `rusty_log` crate (library layer), re-exports only, plus a **baseline** policy check, not a ban. The check covers workspace members only (Nexus is excluded and frozen; third-party transitive dependencies are out of scope). It records the workspace crates that depend directly on `tracing` or `tracing-subscriber` today. It fails if a crate outside the baseline adds such a dependency (the fix is to use `rusty_log`), and fails if a crate on the baseline no longer has one (delete its entry), the same both-ways rule as `PINNED_EXTERNAL_DEPENDENCIES`, so the baseline can only shrink. `rusty_log` itself is exempt. One PR, and it passes its own check because it adds no consumer.
2. Migrate, one PR per area; each must pass its own suite, and none changes a log line. `rusty-mcp` first (it owns the `Layer` and the span code), then the gateway crates, then provider, tailscale and croc, then the remaining crates. Each PR also deletes the migrated crates from the baseline. `rusty-mcp`'s PR also deletes its `tracing` and `tracing-subscriber` entries from `PINNED_EXTERNAL_DEPENDENCIES` in the same change, because the checker rejects a stale entry and counts only dependencies with a `source`. `rusty_log` is a path dependency, so it is **not** added to that list; it only goes into the manifests and `[workspace.dependencies]`.
3. Last PR: the baseline is empty, so replace it with the strict rule (`tracing*` appears only in `rusty_log`), delete the baseline machinery, and close sovereignty item 12 as "chokepoint done; implementation stays third-party until the HTTP stack leaves".

## Decision (2026-10-19)

Option B. The questions below were put to the owner; B is chosen, `rusty_log` is added under `crates/libs/` (the `libs` layer; the owner can ask for a move in review).

## Decision as asked

- **B only (recommended)**, **A (do nothing)**, or **C**?
- If B: is a new crate under `crates/libs/` (alongside `rusty_tokio`) the right home?
- Nexus (excluded, frozen) stays untouched in every option.

## Not covered

Metrics and OTLP export (done in item 9), replacing `axum` and the HTTP stack in the gateway (item 6 covers only `rusty-mcp`), and any change to log content or levels.
