---
category: Changed
changelog: `rusty-mcp` takes its RSA key bounds from `rusty_oauth` instead of keeping its own copy; `rusty_oauth` gains `RsaPublicKey::unusable_reason`.
---
## 2026-10-13 - rusty-mcp, rusty_oauth: one RSA key policy

- **Changed:** `rusty-mcp`'s JWKS key conversion no longer carries its own modulus, exponent and parity checks (and the constants behind them). It builds the `rusty_oauth` key and asks the new `RsaPublicKey::unusable_reason()`, so the policy (modulus 2048 to 4096 bits, no leading zero byte, odd modulus, odd exponent from 3 to 33 bits) lives once, in `rusty_oauth`, where `rusty_channel` and `rleval-app` also get it.
- **Added:** `rusty_oauth::jwt::rsa::RsaPublicKey::unusable_reason() -> Option<&str>`, the check `verify_pkcs1v15_sha256` already made, now readable up front. The text is fixed by the crate and holds only sizes.
- **Behaviour kept:** a bad key is still refused when it is selected, as "unusable signing key: ...", not later as "the token signature is invalid"; no new dependency; no change to which keys are accepted.
- **Behaviour changed (wording only):** the reason texts are `rusty_oauth`'s, so "the RSA modulus must be odd" is now "RSA modulus is even", and the others lose their "the" prefix. One more allocation: the `rusty_oauth` key is built from the decoded components before it is refused, where `rusty-mcp` used to refuse an oversized modulus first. The JWKS body is capped at 1 MiB, so that is bounded.
- **Verified:** the existing `rusty-mcp` key tests pass with the one wording change; a new `rusty_oauth` test pins `unusable_reason`. Removing the accessor's reason (always `None`) was caught in both crates.
- **Known limitations:** `rusty-mcp` still has its own NumericDate claim check, by design (#633 does not change it). The sovereignty note about this gap is updated to say it moved.
