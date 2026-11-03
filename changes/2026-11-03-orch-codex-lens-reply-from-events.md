---
category: Changed
changelog: orch-codex takes a lens question's reply from the `codex exec --json` events instead of a last-message file, so nothing is written to disk and the run can happen inside a container.
---
## 2026-11-03 - orch-codex: lens reply from the JSON events

- **Changed:** a lens question (`Model::ask`) now reads its reply from the last completed `agent_message` item in the `--json` events on stdout, which it already read for usage. `lens_argv` no longer takes a reply path or passes `--output-last-message`, and the private scratch directory is gone. A missing or blank final message is still `Transient`. This prepares Let Em Loose #594's approved deployment: Codex runs in the `lel-codex` container via `podman exec`, where a host-side reply path does not exist.
- **Unchanged:** the role runner (`AgentRunner`) keeps the schema and last-message files.
- **Known limitations:** the event shape (`item.completed` / `item.type = "agent_message"` / `item.text`) is the documented `codex exec --json` schema. It is checked only against fixtures here, because no live call is authorized.
