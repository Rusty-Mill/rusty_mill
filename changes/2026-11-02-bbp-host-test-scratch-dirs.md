---
category: Fixed
changelog: rusty_bbp_host: tests no longer share scratch directories, fixing the review_findings and moderator recovery flakes (#1332).
---
## 2026-11-02 - rusty_bbp_host test scratch directories are unique per call

- **Fixed:** `tests/review_findings.rs` and `tests/moderator_e2e.rs` named their
  scratch directories by tag and process id only. Tests reuse tags (fixture
  names; the `sandbox`/`timeout` recovery case is run by both
  `process_exit_cancels_reaps_and_serializes_a_running_workload_before_restart`
  and `confined_timeout_recovery`), so under the parallel runner one test's setup
  wiped another's live directory. A shared `common::fresh_dir` adds a per-call
  counter. Fixes #1332 and the `confined_timeout_recovery` flake.
- **Known limitations:** the other `rusty_bbp_host` test files keep their own
  tag-named `tempdir` helpers and were not audited for reused tags; none
  has been reported flaky.
