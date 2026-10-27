---
category: Fixed
changelog: The embedder availability cache test can no longer hang the merge queue when the OS reuses a released port.
---
## 2026-10-26 - Embedder cache test cannot hang on a reused port

- **Fixed:** `a_failed_probe_does_not_answer_for_a_different_backend_address` released its "dead" port and then bound the live stub on a fresh ephemeral port. If the OS handed the same port back, the cached failure answered for it, the stub was never probed, and the test blocked forever joining the stub's thread (seen in merge-queue run 40390824325). The live stub now binds first, so the dead port cannot be its port (asserted), and the stub's accept, read and write are bounded so an unexpected cache hit fails the assertion instead of hanging. The cache-invalidation assertions are unchanged.
