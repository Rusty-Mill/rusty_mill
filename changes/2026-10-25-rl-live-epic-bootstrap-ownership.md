---
category: Fixed
changelog: rl_live_test identifies the Epic-launched bootstrap game by its Epic launcher parent, image, start time and Init command line, so live runs can start.
---
## 2026-10-25 - rl_live_test: verify the Epic-launched game by its launcher and command line

- **Fixed:** `Get-RlOwnedBootstrap` required the game to be a direct child of the `cmd /c start` process, which exits at once while Epic's launcher starts the game, so every live run aborted. It now accepts exactly one `RocketLeague.exe` whose parent is `EpicGamesLauncher.exe`, whose image is the expected path, which started after the launch, and whose command line equals the new `Init: Command line:` entry in `Launch.log`. Anything else still aborts without stopping a process. The native handle is retained and its start time compared to CIM within 1 ms (CIM has microsecond precision).
- **Known limitations:** exact command-line equality between `Launch.log` and `Win32_Process` is not yet confirmed on a real machine; a mismatch fails closed. `test-safe.ps1` covers the match and each single-criterion mismatch with mocks only.
