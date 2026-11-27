---
category: Changed
changelog: Documented `rusty_sandbox::signals_scoped()` as kernel capability detection and specified `Sandbox::confine_filesystem_and_signals` in the rustils security behavior spec.
---
## 2026-11-26 - Document sandbox signal scoping

- **Changed:** `rusty_sandbox::signals_scoped()` rustdoc now says it detects the kernel's capability (Landlock ABI v6) and reports nothing about a particular job. rustils `docs/behavior/security.md` gains a `Sandbox::confine_filesystem_and_signals` section: the two statuses, the Linux ABI behavior, the default for backends without a mechanism, and where it is live-verified. Docs only; no behavior change.
