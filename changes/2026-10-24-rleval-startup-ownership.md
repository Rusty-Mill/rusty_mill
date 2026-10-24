---
category: Fixed
changelog: Eliminate the RLEval panel-test startup port handoff race and retain startup diagnostics.
---
## 2026-10-24 - RLEval server startup ownership

- **Fixed:** `rleval serve --port 0` reports its actual address after binding and serves from that same listener. The existing `server::serve` API remains available.
- **Fixed:** Panel tests start the real server without reserving and releasing a port, drain bounded stdout/stderr tails, detect early exit, and probe health with socket deadlines under an elapsed startup deadline.
- **Tests:** Concurrent children retain distinct bound addresses; failed binds never announce readiness; early exit includes status and diagnostics; an unresponsive health peer times out. Existing panel assertions are unchanged.
- **Known limitations:** Lost diagnostics prevent attributing the original queue occurrence conclusively; this fixes the demonstrated ownership race without process retries.
