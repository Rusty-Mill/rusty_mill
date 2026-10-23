---
category: Fixed
changelog: Reuse OpenSSL ACCEPT readiness for the TLS 1.3-only client rejection test.
---
## 2026-10-23 - OpenSSL negative-test startup lifecycle

- **Fixed:** The TLS 1.2 client's TLS 1.3-only OpenSSL rejection test now uses the shared harness with one explicit version flag, ACCEPT readiness, fresh-port startup retries, stderr diagnostics and RAII child cleanup, removing its throwaway TCP readiness probe. The expected peer alert is unchanged.
- **Known limitations:** The scheduled server's exact exit cause was not captured; this repairs the evidenced harness race without claiming a protocol regression. Interop tests still require the OpenSSL binary.
