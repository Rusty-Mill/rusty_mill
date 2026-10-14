---
category: Security
changelog: Update locked rustls from 0.23.45 to 0.23.46 to address RUSTSEC-2026-0346, RUSTSEC-2026-0347, RUSTSEC-2026-0348, and RUSTSEC-2026-0349.
---
## 2026-10-14 - rustls security update

- **Security:** Update the workspace lockfile to rustls 0.23.46, the smallest
  compatible release patched for RUSTSEC-2026-0346, RUSTSEC-2026-0347,
  RUSTSEC-2026-0348, and RUSTSEC-2026-0349. These cover session-secret debug
  output, buffered data after ECH rejection, Brotli certificate-decompression
  allocation, and unauthenticated key-share prediction.
- Existing dependency requirements accept the patched release; no API migration
  or unrelated dependency update is required.
