---
category: Added
changelog: rusty_bbp_host refuses claims-mode candidates that touch protected test paths (#597 slice 3); test changes go through an accepted `tests` claim.
---
## 2026-11-03 - Protected test paths (Let Em Loose #597, slice 3)

- **Added:** the claims-mode profile set may name `protected` globs. The host's Agent API reads each candidate diff's paths as `git apply` does, including the old side of a rename, and refuses `protected_path`, naming each path, when a diff adds, modifies or deletes one. Nothing is stored or debited (owner answer G2: refuse, don't restore).
- **Added:** a claim in the `tests` domain may carry a candidate that touches only protected paths. Once that claim is accepted, other candidates may list its diffs. The core accepts a `tests` claim only with support from a principal who authored neither its candidate nor its diffs.
- **Added:** traces CL-78 (core) and CL-79 (host).
- **Known limitations:** with `protected` set the interim `tests_unendorsed` rule never fires; its fields stay for tasks opened without `protected` and are removed in a later, separate change. A path git quotes in a rename header fails closed.
