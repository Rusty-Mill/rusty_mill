# TLS 1.2 for the native engine: scope and stages

Date: 2026-10-08. Status: in progress. Owner approved implementing TLS 1.2 (decision D1 in
`TLS-ENGINE-ASSESSMENT.md`). Stages 4b-i (record layer) and 4b-ii (key derivation) are built; the rest is not. Supersedes ADR-0002 stage 4b
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
| 4b-ii | PRF, master secret, extended master secret, key block, Finished. **Done**, see below. | OpenSSL's own PRF; captured real OpenSSL handshakes; differential against rustls. | 3 to 4 (took well under one) |
| 4b-iii | Client handshake: ServerHello, Certificate, ServerKeyExchange (signature over randoms and params), Finished, tickets. Reuses `path`/`name`. | Handshakes against rustls (restricted to TLS 1.2), OpenSSL, plus a hostile test server for refusals. | 7 to 9 |
| 4b-iv | Server handshake: mirror, plus client auth. | rustls and OpenSSL clients. | 7 to 9 |
| 4b-v | Version negotiation and downgrade protection in existing client and server; alerts. | rustls both directions; sentinel tests. | 3 to 4 |
| 4b-vi | Fuzz targets and BoGo/tlsfuzzer cases for the 1.2 paths; resource limits. | libFuzzer; BoGo. | 4 to 5 |

Total 27 to 35 days, matching the earlier estimate. Gate for each stage is the ADR-0002
section 5 bar: differential, interop, rejection, fuzz, known-answer tests.

## Risks

- **No RFC 8448 equivalent.** TLS 1.2 has no single published full-handshake trace. Stage
  4b-ii answered this with captured OpenSSL handshakes (below). OpenSSL interop must still
  run in CI from stage 4b-iii (assessment gap G7), not stay `#[ignore]`d.
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

Limits of this evidence: the record layer is not wired to anything; and the mutation and fuzz
figures are single runs. (Real-OpenSSL coverage was added in 4b-ii: it decrypts OpenSSL's own
records with this layer.)

## Stage 4b-ii: done

`handrolled::schedule12`: `prf`, `extended_master_secret`, `key_block`, `finished_verify_data` and
`verify_finished`, for SHA-256 and SHA-384 suites.

TLS 1.2 has no published full-handshake trace (RFC 8448 is TLS 1.3 only), and a derivation
checked only against code that shares this author's reading of the RFC would agree with a
mistake. The strongest oracle available here was a real implementation, so:

| Gate | Result |
| --- | --- |
| Real OpenSSL handshakes | Five captured through a recording proxy (AES-128-GCM, AES-256-GCM, and three ECDHE suites including ChaCha20-Poly1305). From the hello randoms, transcript and pre-master secret this crate derives the extended master secret, key block and `Finished`; the result **decrypts OpenSSL's encrypted Finished records in both directions**, the `verify_data` inside matches, and the first application-data record decrypts to what was sent. For the two RSA key-exchange suites the pre-master secret is recovered by decrypting `ClientKeyExchange`, so the master secret is derived from scratch and equals the one OpenSSL logged. For ECDHE the logged master secret is the starting point, since a capture cannot reveal the shared secret. This also exercises the 4b-i record layer against real OpenSSL records. |
| OpenSSL PRF | 34 vectors from `openssl kdf TLS1-PRF`, which itself matches an independent Python `P_hash`. |
| Differential vs rustls | 4,320 PRF sweep cases (hash, secret, label, seed and output length, around every HMAC block boundary) identical to `Prf::for_secret`. |
| Negative controls | The oracle is required to notice the classic mistakes: client-then-server key block seed, a transcript missing `ClientKeyExchange`, a `Finished` over the wrong messages or with the wrong label, and a SHA-256 hash for a SHA-384 suite. |
| Mutation | 17 deliberate bugs (swapped labels, seed order, keys, IVs, PRF block size, missing label, hash mix-up, unchecked lengths, an accept-anything `verify_finished`). All caught. The seven RFC-reading ones (seed order, EMS label, both Finished labels, key and IV partition, hash choice) were also re-run against **only** the OpenSSL tests and all were still caught, so that oracle stands on its own. |
| Fuzz | New libFuzzer target `schedule12`: pieced seed equals joined, prefix property, key block equals partitioned PRF, `Finished` round trip and bit-flip rejection. 517,084 executions in 90 seconds, no findings. |

Decisions made, inherited by later stages:

- **Extended master secret only.** There is no function for the original RFC 5246 derivation, so
  a handshake cannot call it. A peer that does not negotiate RFC 7627 must be refused in 4b-iii.
- **The caller supplies the transcript hash**, and a hash of the wrong length is an error, so a
  SHA-256 hash can never be silently used with a SHA-384 suite.
- **`Finished` is compared in constant time** (double-HMAC under ring's `hmac::verify`, because
  ring's own comparison helper is deprecated and `hmac::verify` needs a full-length tag).
- **The hash is a parameter, not derived from the AEAD**, because the PRF hash belongs to the cipher
  suite and 4b-iii owns the suite table.

Limits of this evidence: the ECDHE traces cannot check the master-secret derivation itself (the
shared secret is not in a capture), only everything after it; the master-secret derivation is
covered for ECDHE only by being identical code to the RSA case. Secrets are not zeroized on drop,
as in the TLS 1.3 schedule. The mutation and fuzz figures are single runs.

## Next action

Stage 4b-iii, the client handshake: ClientHello with the extended-master-secret and renegotiation
extensions, ServerHello, Certificate, ServerKeyExchange (signature over the randoms and curve
parameters), ClientKeyExchange, `ChangeCipherSpec`, `Finished`. It reuses `path`, `name`, `verify`,
`kx`, `record12` and `schedule12`, and its first oracle is a live rustls (restricted to TLS 1.2)
and OpenSSL server. This is the largest stage; estimate 7 to 9 days, and I would expect the
record and derivation work to have removed some of its risk but not much of its volume.
