# #1382 orch-minted tokens: slice plan

Card #1382 (bbp_factory, M3): orch mints a short-lived token scoped to the lens and claim for every
agent session it launches; the jailed agent reaches tools only through the gateway with it; witness and
gateway digests post on the claim.

Status: **draft, 2026-11-27.** No new design. The behaviour is approved in `726-sandbox-design.md`
(PR #1873, D180; O3 default = D183): §2 token, §9.4 gateway-only egress, §9.6 witness, §10 build order,
§11 ask-first. That approval satisfies D25 for #1382, so this file only cuts the work into slices and
lists the deltas. Crate placement is D209; build order is D212. The plan takes no D or Q number of its own.

**Build order.** §10 puts #1382 last, and the card waits on #1318, #1378 and #1379. Nano confirmed it
(D212, 2026-11-27): only this plan merges now; code starts when #1375–#1379 land.

## 1. What exists

| Piece | Where | State |
|---|---|---|
| ES256/RS256 JWT check against a file JWKS, pinned algorithms, audience list | `agentgateway-auth` `file_jwks.rs` | merged |
| Tool filter by regex and CEL over `jwt.*` on list and call | `agentgateway-mcp` `gate.rs`, `rules.rs` | merged |
| `jail.json` `token` {`file`, `var`} slot; token file kept outside every root | `orch-jail` `spec.rs` (D107) | merged |
| No token in v0 builder launches until #1382 | D187 item 5 | merged |
| `jsonwebtoken` 11.0.0 with `rust_crypto` (p256), `Jwk::from_encoding_key` | `vendor/jsonwebtoken` | merged |
| Session bound to `(iss, sub)` | #1375, PR #1877 (on #1873) | open |
| Lens policy files and `lel.policy` digest check | #1378 | backlog |
| Sandbox fs/shell server | #1379 | backlog |
| Builder launch through `lel-jail` | #1318 | backlog |

## 2. Slices

Each is one small PR with a change fragment, stacked where dependent. Slices 1, 3 and 4 are
security-sensitive (tokens, keys, egress) and get independent review (CONTRIBUTING).

1. **`orch-token` crate, sans-IO** (D209). `Claims` (§2 table: `iss`, `aud`, `sub`, `jti`, `iat`, `exp`,
   `lel.project`, `lel.task`, `lel.claim`, `lel.lens`, `lel.role`, `lel.policy`, `lel.grants`), with
   illegal states refused at construction (empty ids, `exp` before `iat`, role other than
   `builder`/`critic`). `lifetime(budget) -> Duration` = budget + 5 min, capped at 4 h.
   `policy_digest(bytes)` = sha256 hex. `mint(&Claims, &SigningKey) -> Result<String>`: ES256, header
   `kid`. `jwk(&SigningKey) -> Jwk` for the gateway's JWKS. The caller passes `now` and the 128-bit
   `jti`, so the crate does no clock or random I/O. Tests: round trip with `jsonwebtoken::decode`
   under the gateway's settings (ES256 only, `aud = lel-gateway`); wrong `aud`, expired, unknown `kid`
   and an HS256 header are refused; the cap holds; policy digests use test fixtures until #1378.
2. **orch-bbp writes the token** (after #1318). At builder launch: read the key file, mint, write the
   token to a 0600 file outside every `jail.json` root, fill the `token` slot. Never log the token;
   log `jti` and `sub` only. Replaces D187 item 5's "no token". Test: the written `jail.json` passes
   `JailSpec::resolve`, and the token file is outside every root.
3. **Revocation** (gateway). orch appends `{jti, exp}` to `revoked.jsonl` when the claim settles or the
   session is dismissed; the gateway reloads the file on change, refuses those tokens and fires the
   #1377 `session_ended` hook for their sessions. Entries past `exp` are ignored and can be pruned.
   **Ask-first:** the gateway needs a new config field naming the file, which is public API under §11
   but missing from §11's list. This slice brings it to Nano through the expert before any code.
4. **End-to-end** (after #1378, #1379, #1412's egress). A toy agent in `lel-jail` with a minted token:
   a tool call through the gateway succeeds; the same call with no token, a wrong-`aud` token, an
   expired token and a revoked token is refused; a direct connection to any other address fails
   (gateway-only egress, §9.4). The harness is a test crate that depends on both orch-token and the
   gateway, so neither imports the other (D102). This is the card's "Done when".
5. **Witness on the claim** (host, with #1412 item 6). After the session, post the upper-layer diff
   digest and the gateway call-log head (hash-chained, #722) on the claim as non-scoring messages;
   post denials and out-of-scope writes at once; a mismatch with ACP `session/update` posts as `cited`
   evidence in the `counter` direction, citing the call-log artifact (D183). No BBP schema change; if
   one turns out to be needed, it is ask-first.

## 3. Deltas from the design

- **Revocation config field** (slice 3): ask-first, not listed in §11.
- **No keygen code.** The key is a P-256 PKCS#8 PEM made with `openssl genpkey -algorithm EC
  -pkeyopt ec_paramgen_curve:P-256`; orch derives the JWKS entry with `jwk()`. Writing the key file on
  BaileyAi is a §11 host write and needs Nano's yes, given as a paste-ready prompt.
- **Version drift.** The workspace table declares `jsonwebtoken = "9"` while the vendored copy is
  11.0.0; the path patch makes it work. Changing the declared version is a dependency bump, so it is
  left alone here.
