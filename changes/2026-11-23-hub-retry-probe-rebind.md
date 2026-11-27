---
category: Fixed
changelog: Let hub retry tests rebind their probe after server-side TCP connections close.
---
## 2026-11-23 - Rebind hub retry probes after connection shutdown

- **Fixed:** Use SO_REUSEADDR for the loopback probe in the dropped-node hub retry tests so server-side TIME_WAIT sockets do not prevent the retry assertion from running. Keep the existing observation windows and add a regression test for server-first TCP shutdown.
- **Known limitations:** The independent GUI attach retry flake reported in the same nightly sweep has not reproduced locally.
