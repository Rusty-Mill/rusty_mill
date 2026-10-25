---
category: Changed
changelog: msrv-compile also runs on Windows in the scheduled and manual sweeps; its rustc wrapper is now a small Rust program.
---
## 2026-10-25 - msrv-compile on Windows in the nightly sweep

- **Changed:** `msrv-compile` gains a Windows leg on the scheduled and manual full
  sweeps, so a Windows-only dependency that needs a newer rustc than a crate's declared
  `rust-version` is caught (#845). Pull requests and the merge queue still run Linux only.
  The flag-stripping rustc wrapper moved from a POSIX shell script to
  `.github/scripts/msrv_rustc_wrapper.rs`, compiled once per run, because Windows cannot
  run the script. It also filters cargo's `@argfile`s.
- **Known limitations:** macOS is not covered: CI builds no workspace crate there yet.
  The Windows leg leaves out the crates the other Windows legs leave out (rusty_journal,
  rusty_fedora_agent).
