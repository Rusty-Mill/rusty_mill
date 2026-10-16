---
category: Added
changelog: Integrate Orch Implement cards with the existing BBP task host, durable recovery, dynamic rosters and fenced human gates.
---
## 2026-10-16 - Orch BBP task-host integration

- **Added:** optional TaskHost boundary, CLI adapter and goal configuration;
  immutable task-specific pools/rosters/profiles, crash-safe open/adopt,
  unmetered resume, human-only revision-fenced approvals, moderator supervision,
  and completion only after BBP records a merge receipt. References Issue #767.
- **Added:** structured public `bbp show`, transactional human fences and retry
  operations. Existing host/runner/storage infrastructure remains authoritative.
- **Validation:** dispatcher, persistence, CLI contracts, lost replies and a
  scripted two-role real-binary task through receipt; exact results in the PR.
- **Known limitations:** production hosting requires Linux sandbox support.
  Scripted end-to-end workload explicitly runs unconfined; real provider login,
  model behavior and real merges are not exercised.
