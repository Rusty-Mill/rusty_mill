---
category: Changed
changelog: rusty_term and mill-term drop parking_lot for a poison-free wrapper over std::sync::Mutex (`rusty_term::sync::Mutex`).
---
## 2026-10-23 - rusty_term: std mutex instead of parking_lot

- **Changed:** `rusty_term::sync::Mutex` wraps `std::sync::Mutex`, and its `lock()` recovers a poisoned lock, as parking_lot's did, so a panicking holder can't stop the terminal from restoring the host and exiting. Every use in rusty_term and mill-term moves to it, and both drop the `parking_lot` dependency.
- **Changed (public API):** `runtime::run` now takes `Arc<rusty_term::sync::Mutex<Grid>>` in place of `Arc<parking_lot::Mutex<Grid>>`. mill-term is updated, and it was the only caller.
