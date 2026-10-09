# TLS 1.2 for the native engine: scope and stages

Date: 2026-10-08. Status: proposed; owner approved implementing TLS 1.2 (decision D1 in
`TLS-ENGINE-ASSESSMENT.md`). Nothing here is built yet. Supersedes ADR-0002 stage 4b
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
| 4b-i | 1.2 record layer: explicit-nonce GCM, ChaCha nonce, sequence-number AAD, 2^14 limits. | Differential against rustls' 1.2 encrypter; RFC 5288 and 7905 vectors. | 3 to 4 |
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

## Next action

Stage 4b-i. Start with the differential test against rustls' TLS 1.2 record encrypter, then
write the record layer to satisfy it.
