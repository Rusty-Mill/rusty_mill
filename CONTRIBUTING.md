# Contributing

## Before you start

- Match surrounding conventions when editing existing code. This repo has a
  strong house style; the file you are editing is the best guide to it.
- Keep diffs focused — one logical change per PR.
- For large or hard-to-reverse changes (config schema changes, public API
  changes, deletions, dependency or toolchain bumps), open an issue or draft PR
  to discuss first.

## Workflow

1. Branch off the default branch.
2. Make your change. State the *why* in commit messages and the PR description
   for any non-obvious decision — this repo's history is written to be read.
3. Add tests for non-trivial logic — the happy path and at least one failure or
   boundary case.
4. Add or update doc comments on any public surface you touched.
5. Open a PR — pick the template that matches (feature / bug fix / docs / chore).

## Building and testing

The toolchain is pinned in `rust-toolchain.toml`; rustup installs it on first
use. Do not work around the pin — it exists because a clean local run once
passed while CI failed on the same commit, twice, on lints the older toolchain
did not emit.

Run what CI runs, before you push:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Bumping the pin is a deliberate change: raise the version, run the full suite,
and fix whatever the newer lints find in the same PR.

## Code style

- **Explain why, not what.** A comment restating the code earns nothing; a
  comment recording the alternative that was rejected, or the failure mode a
  line prevents, is why anyone can change this later.
- **No `unwrap` or `expect` outside tests.** In tests, `expect` with a message
  saying what was supposed to hold.
- **Errors say where.** Config-facing errors carry an `at` describing the path
  in the file (`binds[0].listeners[1].tls`), because an operator reading it has
  a YAML file open, not a debugger.
- **Refuse at startup rather than at request time** wherever configuration can
  be known bad. A route that answers every call with an error is worse than one
  that never loads.
- **Parse what upstream accepts; enforce what this build can.** A key that
  parses and does nothing must be reported by `Config::lint` — a policy that
  looks like security and isn't is worse than one that fails to load.
- Flat control flow — guard clauses, early returns, avoid deep nesting.
- Minimal dependencies — justify any new one in the PR description. Import
  `rusty_tls`, never `rustls` (see ARCHITECTURE for the one exception and why).
- Never commit or log secrets. Validate external input at the boundary; every
  length in an attacker-supplied message is bounds-checked against the buffer,
  not against the length that preceded it.

## Tests

- **Name the behaviour, not the function.** `a_name_nothing_claims_is_closed`
  beats `test_destination_3`.
- **Prove the thing, don't stub it.** The suite stands up real sockets, real
  subprocess MCP servers, real signed JWTs and real TLS handshakes. A test that
  mocks the mechanism under test tends to pass for the wrong reason.
- **Assert the negative too.** "Did the upstream see it anyway" is a different
  question from "was it refused", and only the second is usually written down.

## Review & merge

- Every change lands through a PR — no direct pushes to the default branch.
- CI must be green before merge. It is not yet a required status check, so this
  is currently a convention rather than an enforced gate.
- Reviewers: check for scope creep, missing tests, unexplained non-obvious
  decisions, and any claim in the PR description that the diff does not support.
- Merge with a **merge commit** ("Create a merge commit" — merge and sync). Do
  **not** squash-merge or rebase-merge: full commit history is preserved
  deliberately.
