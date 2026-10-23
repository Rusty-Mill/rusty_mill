---
category: Fixed
changelog: Avoid Cargo dependency fan-out for prose-only TLS ADR Markdown changes in CI.
---
## 2026-10-22 - Scope TLS ADR documentation impact

- **Fixed:** exclude only direct Markdown files in `crates/libs/net/rusty_tls/docs/adr/` from Cargo-impact seeds after auditing code, build and test consumers. Mixed source, manifest and fixture changes still select TLS and its reverse dependencies; documentation, format, policy, required gates and merge-queue behavior are unchanged.
- **Known limitations:** the exception must be revisited if these ADR files become build or test inputs. Other Markdown, nested directories and non-Markdown files retain their existing impact.
