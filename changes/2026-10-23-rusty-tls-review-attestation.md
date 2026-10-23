---
category: Changed
changelog: "Crypto plan: record the owner's 2026-10-23 attestation that the native TLS backend's remaining reviews were completed."
---
## 2026-10-23 - crypto plan: the owner's review attestation for the native TLS backend

- **Changed:**
  - **Plan §11b:** records the owner's attestation, "all reviews were completed" (#871 comment 6355620568, confirmed by the owner directly). It covers the items §11b listed as having no human review on record:
    - HMAC/HKDF
    - ChaCha20-Poly1305
    - X25519
    - the Montgomery invariants
    - the comparison and wiping helpers
    - the #871 seam
    - AES-GCM
  - **Plan §11 and the D5 entries:** updated to match.
  - **Seam note:** shows #871 as merged, and D5's review question as settled.
- **Not changed:**
  - **Labels:** the automated reviews stay labelled automated.
  - **Unrecorded:** the reviewed commits, the method and any findings.
  - **Not a certification:** this is not a timing or production-security certification.
  - **D5 export gating:** whether `rusty_aead`'s AES types stay exported unconditionally is still open.
