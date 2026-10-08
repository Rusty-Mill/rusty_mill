# TLS 1.2 for the native engine: scope and stages

Date: 2026-10-08. Status: in progress. Owner approved implementing TLS 1.2 (decision D1 in
`TLS-ENGINE-ASSESSMENT.md`). Stage 4b-i (record layer) is built; the rest is not. Supersedes ADR-0002 stage 4b
("declined") once the first stage lands and the crate ADR is amended (decision D3, still open).

## Principle

Implement the smallest TLS 1.2 that interoperates with current servers and clients, and
refuse everything else. Fewer suites means fewer ways to be wrong. Every refusal below is a
test, not an omission.

## In scope

| Item | Choice |
| --- | --- |
| Roles | Client and server. The seam has both (`TlsStream`, `TlsAcceptor`). |
| Key exchange | ECDHE only (X25519, P-256, P-384). Reuses `kx`. |
| Authentication | ECDSA and RSA certificates. RSA signs ServerKeyExchange with PSS or PKCS#1 v1.5, as RFC 5246 allows. Reuses `verify`, `path`, `name`, `sign`. |
| Ciphers | AEAD only: AES-128-GCM, AES-256-GCM, ChaCha20-Poly1305 (RFC 5288, 7905). |
| PRF | HMAC-SHA256 or SHA384 per suite (RFC 5246 section 5). |
| Extended master secret | **Required** (RFC 7627). A peer that omits it is refused. |
| Secure renegotiation | `renegotiation_info` sent and checked (RFC 5746); renegotiation itself refused. |
| Resumption | Session tickets (RFC 5077) after the base stages pass. Session IDs refused. |
| Version negotiation | Existing 1.3 client offers 1.3 and 1.2; downgrade sentinel (RFC 8446 section 4.1.3) checked both ways. |

## Refused, with a test each

CBC suites and MAC-then-encrypt, RC4, 3DES, static RSA key exchange, finite-field DHE, SHA-1
signatures, compression, renegotiation, session-ID resumption, missing extended master
secret, TLS 1.0 and 1.1, SSLv3, client-side RSA key exchange.

## Stages

Each stage is independently reviewable and abandonable, as in ADR-0002.

| Stage | Scope | Oracle | Days |
| --- | --- | --- | --- |
| 4b-i | 1.2 record layer: explicit-nonce GCM, ChaCha nonce, sequence-number AAD, 2^14 limits. **Done**, see below. | Differential against rustls' 1.2 encrypter; independent vectors. | 3 to 4 (took well under one) |
| 4b-ii | PRF, master secret, extended master secret, key block, Finished. | RFC 5246 PRF vectors from published test suites; differential against rustls. | 3 to 4 |
| 4b-iii | Client handshake: ServerHello, Certificate, ServerKeyExchange (signature over randoms and params), Finished, tickets. Reuses `path`/`name`. | Handshakes against rustls (restricted to TLS 1.2), OpenSSL, plus a hostile test server for refusals. | 7 to 9 |
| 4b-iv | Server handshake: mirror, plus client auth. | rustls and OpenSSL clients. | 7 to 9 |
| 4b-v | Version negotiation and downgrade protection in existing client and server; alerts. | rustls both directions; sentinel tests. | 3 to 4 |
| 4b-vi | Fuzz targets and BoGo/tlsfuzzer cases for the 1.2 paths; resource limits. | libFuzzer; BoGo. | 4 to 5 |

Total 27 to 35 days, matching the earlier estimate. Gate for each stage is the ADR-0002
section 5 bar: differential, interop, rejection, fuzz, known-answer tests.

## Risks

- **No RFC 8448 equivalent.** TLS 1.2 has no single published full-handshake trace. The
  independent oracles are the PRF vectors and OpenSSL, so OpenSSL interop must run in CI
  from stage 4b-iii (assessment gap G7), not stay `#[ignore]`d.
- **Downgrade surface.** Speaking 1.2 gives up the property that the client cannot be
  downgraded because it speaks nothing older. The sentinel, `renegotiation_info`, and
  extended-master-secret requirement replace it. They need adversarial tests first.
- **The F1 name-constraint bug** (assessment) blocks any default status regardless of 1.2.

## Stage 4b-i: done

`handrolled::record12`: `Sealer` and `Opener` for AES-128-GCM, AES-256-GCM and
ChaCha20-Poly1305, with the nonce, additional data and limits TLS 1.2 uses.

Evidence, against the ADR-0002 section 5 bar:

| Gate | Result |
| --- | --- |
| Differential vs rustls | Byte-identical sealing over 1,568 cases (2 algorithms, 14 lengths, 14 sequence numbers, 4 types), and each side opens the other's records. Run again with rustls' real behaviour of a non-zero starting explicit nonce, where the wire nonce differs from the sequence number. |
| Independent known answers | 14 vectors produced by Python `cryptography` (OpenSSL) from the RFC text, covering all three algorithms **including AES-128-GCM**, which rustls cannot reach. Generator is `tests/data/record12_vectors.py`. |
| Rejection | 23 tests. The centrepiece flips every bit of a valid record for each algorithm and requires the exact error for each region (type, version, length, nonce, ciphertext, tag). |
| Mutation | 17 deliberate bugs (wrong AAD, wrong nonce, unchecked version, unchecked limits, sequence wrap, and so on) were applied one at a time. All 17 were caught. Script is not kept in the repo; this is a one-off measurement. |
| Fuzz | New libFuzzer target `record12_open` (arbitrary bytes never panic; seal then open is the identity). 5.2 million executions in two minutes, no findings. Not a long run. |

Decisions made, which later stages inherit:

- **Explicit nonce is the sequence number**, as RFC 5288 recommends. The opener reads it from
  the wire and never assumes it, so rustls' different choice interoperates.
- **Version must be exactly `0x0303`** on protected records. TLS 1.2 authenticates the version
  bytes, unlike TLS 1.3, which ignores them.
- **Plaintext limit is checked before decrypting**, so an oversize record costs no crypto.
- **Unknown content types pass through** for the layer above to refuse.
- **Key and sequence stay paired** (`new_at`), with a documented warning that resuming a
  GCM key at a used number leaks the authentication key.

Limits of this evidence: there is no real-server interop yet (that is stage 4b-iii); the record
layer is not wired to anything; and the mutation and fuzz figures are single runs.

## Next action

Stage 4b-ii: the PRF, master secret, extended master secret, key block and Finished. Its oracles
are published PRF vectors and rustls. Nothing in 4b-i changes it.
