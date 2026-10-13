---
category: Changed
changelog: CI now runs the `library/` unit tests and builds the library page in a `library unit tests` job that `required-gate` waits on.
---
## 2026-10-13 - Run the `library/` tests in CI

- **Changed (CI):** new `library-tests` job (`library unit tests`) in `ci.yml`: `python3 -m unittest` in `library/` (9 tests: frontmatter parser, required `source`, optional fields, skill export round trip, all five kinds load) and `python3 library/build.py`. Stdlib Python only, no cargo; runs for every event like `plan-tests`, and `required-gate` now lists it in `needs`.
- **Changed (tests):** `test_ci_workflow` exempts `library-tests` from the per-job concurrency-lock rule, as it does `plan-tests`: the job starts for every SHA and has no scoped work to retain.
- **Verified (run):** `python3 -m unittest discover -s .github/scripts -p 'test_*.py'` (192 tests) and the library tests pass locally; the workflow parses and the job is in the gate's `needs`.
- **Known limitations:** `actionlint` was not run locally (CI's `workflow lint` runs it). The `backup-before-edit` mod tests (`claude plugin test`) need the `claude` CLI and are still author-run, as are the headless-browser checks of the UI. A change that touches only `library/` still gets the full set of cheap always-on jobs; nothing is scoped by path.
