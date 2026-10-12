---
category: Changed
changelog: `rusty_oauth` refuses RS256 verification with an RSA key whose modulus is outside 2048 to 4096 bits or whose exponent is unusable.
---
## 2026-10-12 - rusty_oauth: RSA key bounds in RS256 verification

- **Changed:** `jwt::rsa::verify_pkcs1v15_sha256` and `verify_rs256` refuse an `RsaPublicKey` whose modulus is not 2048 to 4096 bits, has a leading zero byte (RFC 7518 §6.3.1.1), or whose public exponent is even, below 3 or over 33 bits. They used to check only that the modulus was large enough to hold the digest (62 bytes), so by reading the code a 512-bit key was accepted and there was no upper bound on the modulus a `jwks_uri` could supply; neither case was exercised before this change.
- **Why these numbers:** 2048 is the RFC 7518 §3.3 minimum for RS256. 4096 is a cost cap, not an RFC rule; a provider that publishes a larger key would now be refused. The exponent limits are the ones `rusty-mcp` already enforced on top of this crate.
- **Not changed:** `RsaPublicKey::from_components` and `from_jwk_base64url` still never fail; the refusal happens when the key is used to verify, with a `Validation` error naming the reason. New public constants `jwt::rsa::MIN_MODULUS_BITS` and `MAX_MODULUS_BITS`.
- **Affects:** the other callers of this verification, `rusty_channel`'s Teams verifier and `rleval-app`'s OIDC sign-in, now refuse a key outside these bounds instead of accepting it. Their tests pass; whether the providers they talk to publish keys inside the bounds was not checked.
- **Verified:** six new unit tests pin the exact boundaries (2047, 2048, 4096 and 4097 bits; exponents 1, 2, 3, 65537, 2^33 - 2, 2^33 - 1, 2^33 + 1, six bytes, and a nine-byte value that wraps to 3). Each protection was broken in turn and caught: both bounds, the leading-zero check, exponent evenness, the floor of 3, the 33-bit cap, the length guard before the exponent is folded, and the check in `verify_pkcs1v15_sha256` itself.
- **Known limitations:** the bounds are fixed constants, not configurable. No key outside 2048 to 4096 bits was tested against a real provider. `rusty-mcp` still carries its own copy of the checks; removing it is a separate change.
