---
category: Fixed
changelog: msrv-compile also checks each crate with `--all-features` at its declared rust-version; eleven floors raised to what their features need.
---
## 2026-10-25 - msrv-compile checks all features at the declared floor

- **Fixed:** `msrv-compile` checked each crate at its declared `rust-version` with default
  features only, so an optional feature could need a newer rustc unnoticed (#844). A crate
  with features is now checked a second time with `--all-features`. That found eleven wrong
  floors, now raised: rusty_hyper 1.88, rusty_http 1.85 (and rusty_serve and rusty_agui 1.85 with it), replay-pacifist 1.82, rusty-search
  1.90, rleval-app 1.89, rusty-acp 1.94, rusty_llama 1.90.
- **Known limitations:** five crates keep a floor that covers their default build only, by
  design, and are checked with default features: platform-linux (`track-p`), rusty_json
  (the optional derive), rusty_tls (`native-crypto`), rusty_rdp and rusty_tonic (optional
  transports). Windows and macOS are still unchecked (#845).
