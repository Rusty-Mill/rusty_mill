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

- **Fixed in review:** per-task work isolation; correctable rejected human input;
  expired-request recovery; Unix-only manifest directory fsync; supervised runner
  cancellation/reaping and serialization across moderator timeout, cancellation
  and abrupt death. Added concurrent-task, corrected-answer, expired-request and
  process-level running-workload regressions. Call accounting is not a cost cap;
  PR #763 remains separate.
- **Fixed in safety review:** the shared sandbox executor now owns every spawned
  helper through setup failure, containment and reaping. BBP registers owner-EOF
  cancellation before setup polling. Delayed-helper regressions cover forced
  startup timeout, registration/resolution/status failures, unwaited-job cleanup,
  and owner EOF with task-lock retention and no later workload launch. The
  production setup deadline and confinement policies are unchanged.
