---
category: Added
changelog: "`rusty_orch lens check` loads a lens config and the operator's assertions, builds every provider and reports which would be admitted, without calling a model."
---
## 2026-11-03 - rusty_orch: `lens check` command

- **Added:** `rusty_orch lens check <config.json> [--assertions <file>] [--repo <dir>]` (Let Em Loose #594). It parses the lens config, reads the operator's assertions (default `/etc/let-em-loose/assertions.json`) and builds the provider registry, so a bad setting fails with its field named. It then prints one line per provider, by name: its kind and whether it would be admitted. Examples: "ready if logged in as <account>", "ready in container lel-codex at /work if it checks out at each call", "unavailable: …". An assertions file that cannot be used is reported and treated as none, so the check fails closed.
- **Unchanged:** it calls no model and inspects no container. Logins and the container are still checked before every question.
