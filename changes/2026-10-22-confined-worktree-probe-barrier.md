---
category: Fixed
changelog: Bound and synchronize the shared-worktree confinement test so a fast harness cannot remove its checkout before its peer probes it.
---
## 2026-10-22 - Synchronize shared-worktree confinement probes

- **Fixed:** hold both fixture harnesses at an explicit host-controlled barrier until both finish their confinement probes. Force the second participant to start probing after the first finishes, retain all security assertions, and diagnose bounded waits with participant logs and paths.
- **Known limitations:** the fixture regression runs without a sandbox; confinement assertions still require the enforcing Linux acceptance job. No runtime confinement or provider behavior changes.
