---
category: Added
changelog: remind-me proposes project stages and anchors from captured session context, for a person to confirm.
---
## 2026-10-26 - Project proposals and backfill (ADR-0027 slice 2)

- **Added:** `remind_me_core::project_proposals`. Five deterministic rules read what sessions already captured and suggest project claims: mentions or a project-ish entity kind with nothing built, a git remote whose repository name matches the project, a working directory under `crates/<group>/<name>`, a recorded design decision for an idea, and sixty days of silence (configurable through `REMIND_ME_PROJECT_DORMANT_DAYS`). Proposals are computed on read, never stored, and carry the memories they read as the evidence a confirm cites. No model is called.
- **Added:** `confirm` writes the claim through `project::set`; `reject` records the proposal's key as a `work_log` memory so it is not offered again, while a different value for the same predicate still is. Nothing promotes on its own.
- **Added:** `remind_me_core::project_backfill`. It walks a checkout's crate directories and, for each one matching a known entity, saves one `reference` memory naming the repository and path, which the rules then read as ordinary evidence. A dry run writes nothing; nothing is migrated.
- **Added:** a repo or path proposal for an entity with no stage yet carries `stage = built, status = active`, so confirming it alone creates the project. Proposal keys keep canonical values, so a rejected `crates/Foo` does not hide `crates/foo`.
- **Security:** a secret-shaped `cwd`, `git_remote` or entity name never becomes a proposal, and backfill refuses secret-shaped crate paths (reported redacted) and repository URLs. Rejection and seed memories go through the common scrubber.
- **Added:** an entity whose project claims are invalid gets no proposals; a storage error fails the read instead of looking like an absent project.
- **Known limitations:** no tool, HTTP route or dashboard view yet (slice 3). A project with two different remotes, or two candidate paths, proposes nothing: that contradiction is for a person to settle with `set`.
