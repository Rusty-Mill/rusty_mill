---
category: Security
changelog: Update Hister's Ammonia sanitizer to 4.1.8 to fix RUSTSEC-2026-0373.
---
## 2026-10-17 - Patch Ammonia CSS URL filtering

- **Security:** update `rusty-hister-extractor` from Ammonia 4.1.7 to 4.1.8, the smallest patched release for RUSTSEC-2026-0373, and constrain the dependency to the patched 4.1 release line. CSS URL scheme filtering now applies to quoted `URL()` functions regardless of casing.
- **Tests:** cover denied URL schemes with lowercase, uppercase and mixed-case CSS functions in both sanitizer policies, and preserve allowed URLs, safe styles and the trusted-layout boundary. The scheme-filter regression fails against 4.1.7.
