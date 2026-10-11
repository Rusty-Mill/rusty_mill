---
category: Added
changelog: AES-GCM design note (draft, no code) for the reopened gap 3: portable table-free vs intrinsics, evidence bar, gated order of work, open questions for the owner.
---
## 2026-10-10 - AES-GCM design note (draft)

- **Added (docs):** `docs/research/AES-GCM-DESIGN.md`: portable bitsliced AES plus multiply-based GHASH versus AES-NI/PCLMULQDQ, with a recommendation to build the portable version first and decide intrinsics with a measurement in hand; the evidence bar (Wycheproof and spec vectors, differential against `ring`, valgrind taint with a planted table-lookup control, pinned jump counts, timing tests); a gated order of work whose first step measures throughput; open questions for the owner (D1, performance bar, scope, ticket nonce).
- **Known limitations:** nothing is implemented. Throughput figures are the plan's estimates (60 to 150 MB/s portable against `ring`'s 9.1 GB/s), not measurements. The design assumes 64-bit integer multiply is data-independent on x86-64 and aarch64 and says so. The only check run for this note is that the repo's `valgrind_selftest.sh`, including its secret-indexed-lookup control, passes. The intrinsics approval (D1) is not reconfirmed.
