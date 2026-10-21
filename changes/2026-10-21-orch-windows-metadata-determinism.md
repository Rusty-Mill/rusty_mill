---
category: Changed
changelog: Add bounded diagnostics for investigating Windows Cargo metadata instability.
---
## 2026-10-21 - Windows impact revision diagnostics

- **Changed:** Opt-in, content-free Cargo metadata fingerprints now help diagnose non-deterministic change-impact reports. This instrumentation does not yet identify or correct the Windows instability.
