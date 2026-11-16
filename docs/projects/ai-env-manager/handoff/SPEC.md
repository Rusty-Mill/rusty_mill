# Ai Env Manager — spec
Design chat: https://claude.ai/artifact/Fr2X16gcc9NJJQDPRMzKpQ (Claude Doc, 2026-11-12; copy in `sources/skills-tool-recommendation.md`) · remind-me tag `ai-env-manager`
Owner: Nano (`baileyrd`) — the person whose words count as `[stated]`
Tags: [stated] [default] [open]. Pack root: `docs/projects/ai-env-manager/handoff/`.

## Status
Priority tier Normal: active, one change in flight (DECISIONS.md D16, supersedes D13).
M0 cards (core and linker) and the crate scaffold (D11) may start after the overlap check
(OPEN_QUESTIONS.md Q7). New third-party dependencies stay ask-first (D12).

## Intent
One tool that manages the AI working environment Nano already keeps in git: skills,
plugins/mods, agents, prompts and the key Markdown files (CLAUDE.md, AGENTS.md sections).
It catalogs them from their sources, links them into each AI tool (Claude Code, Codex,
Cursor, Gemini CLI, …) per entry and per tool, scans them before install, and repairs
drift. Built small and safe, because it writes into other tools' config folders.

## Scope
- In: catalog of entries of every kind (D2); sources linked in place from git repos and
  folders (D5); per-entry, per-tool enable/disable with link fallback (D6); adopt
  existing entries; `doctor`/`fix` for drift; risk scanner gating installs; profiles;
  lint and context-cost report; MCP server over the same core.
- Out: forking Skills Manager or Skills Desktop (D3); built-in editor; LLM translation;
  OAuth, vendor marketplaces as built-ins, telemetry, auto-updater; whole-folder
  linking; any write outside our own folders without a previewed plan (D8).
- `library/` is absorbed over time: its entry kinds and `sources.json` are the domain
  model; `build.py` runs until the Rust core renders the catalog, then retires (D14).
- Undecided: desktop shell (Q3); how Markdown sections are applied (Q6).

## Constraints
- Lives in rusty_mill, named "Ai Env Manager" (D1). [stated]
- Covers skills, plugins, agents, mods and key md files, not only skills (D2). [stated]
- Standing preferences: small, modular, explicit; ports-and-adapters with an I/O-free
  core; no `unwrap`/`expect` in library code; minimal dependencies; async only for real
  I/O concurrency (CLAUDE.md, remind-me preferences).
- Repo rules: AGENTS.md (PRs, change fragments, no agent merges; new work needs a priority).

## Shape
```
 front ends      CLI (first)      MCP server (M4)     desktop shell (Q3)
                      \               |                 /
 core (no I/O)   Entry · Source · Tool · Profile · Link · Plan · Finding
                 planner · scanner rules · lint · cost   (pure, tested with fakes)
                      /           |            |           \
 adapters        fs linker     git source   state store   tool catalog (TOML)
                 (symlink →
                  junction →
                  marked copy)
```
Front ends never call adapters directly; everything goes through the core, so all
front ends share the same safety rules (D4). Mutations are a `Plan` first, applied only
with explicit consent (D6). Home when code starts: `crates/apps/ai_env_manager/` (D11).

## Acceptance — first slice (M0: core and linker)
- [x] Core crate with `Entry` (kind = skill to start), `Source`, `Tool`, `Link`, `Plan`;
      no I/O, unit-tested. (`ai_env_core`, D19)
      Ids are lower-case `[a-z0-9._-]`, no leading or trailing dot, no Windows device
      names; a hand-edited managed copy is refused, never replaced (D20).
- [x] Tool catalog as TOML data, seeded with the tools Nano uses (Q4 interim: Claude Code,
      Codex, Cursor, Gemini CLI). (`ai_env_core`, D25: Claude Code and Codex so far;
      Cursor and Gemini CLI wait until their skills folders are verified.)
- [x] CLI `list`, `enable`, `disable` producing a plan; `--yes` applies. (`ai_env_cli`, D26;
      skills come from `--skills DIR` until sources land in M1.)
- [x] Linker: symlink, then junction (Windows), then marked copy; managed-path marker.
      (`ai_env_fs`, D23; the junction path is not run in CI yet.)
- [ ] Tests: enable/disable round-trip on Linux and Windows CI; an unmarked folder is
      never deleted or replaced.
      (Linux half done: planner tests, D19, and on-disk round-trip tests in `ai_env_fs`,
      D23. Windows CI still waits.)

Later milestones (M1 sources and adoption, M2 scanner and gated install, M3 profiles and
lint, M4 MCP then desktop) are in `sources/skills-tool-recommendation.md` § Build plan.
