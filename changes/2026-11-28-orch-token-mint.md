---
category: Added
changelog: orch gains `orch-token`, the sans-IO mint for the short-lived ES256 token each jailed agent session will carry to the gateway.
---
## 2026-11-28 - orch: `orch-token` mint

- **Added:** `orch-token` (Let Em Loose #1382 slice 1, 726-sandbox-design §2, D180, D209). It turns one session's scope into an ES256 JWT the gateway's `FileJwks` accepts, and gives the matching public JWKS entry.
  - Claims: `iss` `lel-orch@<host>`, `aud` `lel-gateway`, `sub` `<project>/<claim>/<lens>/<n>`, a 128-bit hex `jti`, `iat`, `exp`, and `lel.project`, `lel.task`, `lel.claim`, `lel.lens`, `lel.role`, `lel.policy` and `lel.grants` (left out when empty). Empty ids, a `/` in any part of `sub`, an empty grant and an `exp` that overflows are refused when the claims are built; the role is an enum, so it is `builder` or `critic`.
  - `lifetime(budget)` is the budget plus 5 minutes, capped at 4 hours. `policy_digest` is the same lowercase sha256 hex the gateway's lens policy check computes.
  - `SigningKey::from_pem` takes a P-256 PKCS#8 PEM key and refuses any other curve at load. Its `Debug` shows only the `kid`.
  - Sans-IO: the caller passes the key bytes, `now` and the `jti`. No new package in `Cargo.lock`: it uses the vendored `jsonwebtoken` 11 with the features `agentgateway-auth` verifies with.
- **Known limitations:** Nothing calls it yet. orch-bbp writes the token into `jail.json` in slice 2, and revocation (slice 3) waits on Nano's approval of a new gateway config field. The tests check tokens with `jsonwebtoken` under the gateway's settings rather than through the gateway itself; the end-to-end check is slice 4.
