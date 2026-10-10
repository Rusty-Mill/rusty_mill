---
category: Added
changelog: "**`rusty_orch`: ADR-0013 (Proposed), the BBP task host.** An `Implement` card opens one Blackboard Protocol task through a new `TaskHost` port; rusty_orch is the BBP human principal, gates become Board questions, and `orch-bbp` drives the `bbp` CLI. Design only."
---
## 2026-10-10 - rusty_orch: the BBP task host design (ADR-0013)

- **Added (docs):** `crates/apps/rusty_orch/docs/adr/0013-bbp-task-host.md`, Proposed. Answers the question the BBP decision record left for stage 3: the CLI adapters do not become BBP agents and the dispatcher does not proxy them; the BBP moderator launches the vendor CLIs, and rusty_orch acts as the task's human principal through a second dispatcher port, `TaskHost` (`open`, `poll`, `relay`), with `Progress::{Working, Gate, Closed, Cancelled}` and a new `Outcome::Waiting`. The task handle is an `Artifact` entry on the Board, so persistence and `orch-core` are untouched; BBP gates surface as `Question` entries answered with `bbp human` verb lines relayed with the card revision. The adapter crate `orch-bbp` drives the `bbp` binary via `orch_cli::CommandRunner` and decodes `bbp card` with `rusty_bbp`; the workspace layer check forbids linking `rusty_bbp_host`.
- **Known limitation:** design only, awaiting approval; implementation is four PRs (port and fake, adapter and e2e, goal file and flags, Board slimming docs). Per-crate `CHANGELOG.md` and `RELEASE_NOTES.md` of `rusty_orch` carry the same entry.
