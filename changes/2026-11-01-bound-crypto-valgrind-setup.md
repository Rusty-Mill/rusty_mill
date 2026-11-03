---
category: Fixed
changelog: Bound crypto CI valgrind setup with acquisition retries, network timeouts and a blocking step timeout.
---
## 2026-11-01 - Bound crypto valgrind setup

- **Fixed:** the crypto constant-time job retries failed apt acquisitions up to three times with 30-second HTTP/HTTPS timeouts, fails on incomplete index updates, and limits the install step to ten minutes. Setup failures still block the crypto checks and required gate.
- **Known limitations:** acquisition timeouts cover connections and inactive transfers, not the whole apt operation. The step timeout bounds total setup; this does not establish the cause of the observed stall or increase runner capacity.
