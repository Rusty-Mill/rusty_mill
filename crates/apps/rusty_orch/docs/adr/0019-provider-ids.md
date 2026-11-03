# ADR-0019: Providers are ids, not an enum

- **Status:** Accepted (Let Em Loose #594, owner decision 2026-11-02)
- **Date:** 2026-11-03

## Context

`orch_core::task::Agent` was a closed enum `{Claude, Codex, Gemini, Local}`.
Adding a model backend meant a change to the pure core, and the lens runner
(#594) needs providers chosen by configuration: two local models
(`local`, `local-high`), Codex, Claude, and later Gemini over HTTP.

## Decision

- `Agent` becomes `AgentId`: a validated token of 1 to 32 bytes from
  `[a-z0-9_-]`, like a BBP role id. It is stored inline, so it stays `Copy`.
  Core holds no list of providers.
- Each adapter crate exports the id it serves by default (`orch_claude::ID`
  is `claude`, `orch_codex::ID` is `codex`, `orch_ollama::ID` is `local`).
  The `rusty_orch` binary keeps the set it can route (`agents::KNOWN`, which
  also names the adapterless `gemini`) until the config-driven registry
  replaces it.
- **Persistence.** `orch-store` keeps schema tag v1. `AgentRow` gains a fifth
  variant, `Id(String)`, appended so the positional tags of the four old
  variants still decode. New snapshots write only `Id`. Old variants are
  read as `claude`, `codex`, `gemini` and `local`. Stored history is never
  rewritten; it changes form only when a resumed goal is saved again.
- Errors and progress lines print the id (`codex failed T-1`), not the old
  variant name (`Codex failed T-1`).

## Consequences

- A new provider is a config entry, plus an adapter crate only for a new
  protocol.
- A snapshot written by this version cannot be read by an older binary: it
  sees an unknown variant and reports the record corrupt. Rolling back
  `rusty_orch` past this change means discarding or not resuming goals
  saved after it.
- An invalid stored id is reported as corrupt, like any other bad row.
