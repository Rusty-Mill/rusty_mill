---
category: Added
changelog: "`rusty_tls`: BoGo runs against the native-backend engine (client role), with its own CI floor."
---
## 2026-10-23 - rusty_tls: BoGo on the native backend (client role)

- **Added:**
  - The BoGo shim gains `ring` (default) and `native` features.
  - On the native backend (`--cfg rusty_tls_native_crypto`, ADR-0005), the server role, ticket key and client certificates are compiled out of the shim. Those scenarios exit 89, so they count as unimplemented, not failed.
  - `bogo/run.sh` gains `NATIVE=1`, with its own target and results directories.
- **CI:** the `rusty-tls-engine-native` job runs BoGo with its own floor of 430 (445 passed, 0 failed at the pinned BoringSSL commit). The `ring` job's BoGo run and its floor (830) are unchanged.
- **Docs:** this closes the native BoGo deferral recorded in ADR-0005 decision 4 and the seam note's §4.
- **Not changed:** `rusty_tls`'s own code and version. The shim is an unpublished harness.
