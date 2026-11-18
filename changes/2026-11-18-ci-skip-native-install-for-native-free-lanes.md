---
category: Changed
changelog: CI clippy and test lanes whose crates link no GTK, X11, D-Bus or ALSA code skip the native apt install that timed out on mirror downloads.
---
## 2026-11-18 - CI skips the native apt install for native-free lanes

- **Changed:** `ci_components.py` marks each clippy/test lane `native` when its packages' dependency closure reaches a crate that needs the apt packages in `native-packages.txt`. `setup-build-env` takes a `native` input and, when `false`, skips the apt install, its download cache and the D-Bus keyring. The Mill Board lane and 33 other lanes no longer download GTK/WebKit/GStreamer packages; full sweeps and the other jobs install as before.
- **Known limitations:** the list of native crates (`NATIVE_CRATES`) is maintained by hand; a lane that gains a native dependency the list misses fails loudly at its build step.
