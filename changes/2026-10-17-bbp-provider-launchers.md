---
category: Added
changelog: Add turn-bound Codex author and MCP-only Claude reviewer launchers, offline contracts and an opt-in Linux provider smoke.
---
## 2026-10-17 - BBP provider launchers

- **Added:** runnable `codex-bbp-harness` and `claude-bbp-harness` binaries in
  the existing BBP host package, with current-turn/principal/vendor checks,
  exact per-turn MCP configuration and fresh provider sessions.
- **Added:** separate author checkouts, required Codex MCP configuration,
  enforced Claude built-in tool removal, exclusive BBP MCP configuration,
  no provider-flag passthrough and exec-based process ownership.
- **Validation:** offline argument/policy and real-process contracts, plus an
  explicitly opted-in Linux subscription-provider lifecycle smoke through
  confined validation, independent review and a test-only receipt. No real merge.
- **Known limitations:** Linux launchers, compatible authenticated provider CLIs
  required; deterministic contracts do not claim successful live model behavior.
- **Fixed:** per-tool Codex MCP approval rules and inherited Git repository/index
  contamination, with provider-stub regressions.
- **Added:** replay-derived, provider-neutral no-progress launch protection;
  pauses survive moderator/Orch restarts and require a current-revision owner
  reset. No event-log/schema changes; only an acknowledged reset revision is
  host metadata. Added process and real-Orch recovery regressions.
- **Added:** a pre-task Rust/LLD profile generator with one linker thread under
  the existing 1 GiB cap, plus a fifth required confined acceptance case.
- **Validation:** real deadline expiry pauses and reaps live harnesses, retaining
  that pause on restart. Required CI explicitly runs the Orch pause/adoption
  regression and records its evidence separately from the five confined cases.
