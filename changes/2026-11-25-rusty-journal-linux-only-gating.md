---
category: Fixed
changelog: `rusty_journal` now builds on Windows and macOS by cfg-gating its io_uring modules to Linux; Windows CI builds it.
---
## 2026-11-25 - rusty_journal builds on non-Linux targets

- **Fixed:** `client`, `consumer`, `manifest`, `retention`, `segment`, `server` and the server binary imported io_uring items that `rusty_tokio` exports only on Linux, so the crate failed with E0432 on Windows and macOS. They are now `cfg(target_os = "linux")`; `clock`, `offset`, `protocol` and `record` remain available everywhere. The binary exits with a clear message on other OSes. Linux behaviour is unchanged. CI no longer excludes `rusty_journal` from the Windows legs.
- **Known limitations:** the journal itself remains Linux-only; there is no portable std-fs fallback (owner decision, card #926).
