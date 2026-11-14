---
category: Changed
changelog: Ai Env Manager moves its load, plan and apply steps out of the aienv CLI into a shared ai_env_app crate, with no change to CLI behaviour.
---
## 2026-11-14 - Ai Env Manager: shared ai_env_app crate (D32)

- **Changed:** the steps `aienv` ran inline (select tools, load skill entries, observe, plan, apply) now live in the new `ai_env_app` crate as plain functions returning data and errors. `ai_env_cli` only parses arguments and prints. The planned GUI will call the same crate, so both front ends share one set of safety rules. `ai_env_cli::Error` and `Scope` are re-exported, so the public CLI surface and its output, exit codes and error text are unchanged; the existing CLI tests pass unmodified.
- **Known limitations:** no GUI yet. `ai_env_app` is skills-only and takes `--skills` style folders, like the CLI (sources arrive in M1).
