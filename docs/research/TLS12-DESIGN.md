# TLS 1.2 for the native engine: scope and stages

Date: 2026-10-08. Status: in progress. Owner approved implementing TLS 1.2 (decision D1 in
`TLS-ENGINE-ASSESSMENT.md`). Stages 4b-i (record layer), 4b-ii (key derivation) and 4b-iii (client handshake) are built; the server, version negotiation and hardening are not. Supersedes ADR-0002 stage 4b
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
| 4b-iv | Server handshake: mirror, plus client auth. **Done**, see below. | rustls and OpenSSL clients. | 7 to 9 (took well under one) |
| 4b-v | Version negotiation and downgrade protection in existing client and server; alerts. **Done**, see below. | rustls both directions; sentinel tests. | 3 to 4 (took well under one) |
| 4b-vi | BoGo cases for the 1.2 and negotiation paths; resource limits. **Done**, see below. | libFuzzer; BoGo; mutation. | 4 to 5 |

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

## Stage 4b-iii: done

`handrolled::client12` (state machine and connection), `handrolled::handshake12` (the TLS 1.2
messages), and `verify_tls12_signature` in `handrolled::verify`. A sans-IO ECDHE client for the six
AEAD suites, offering TLS 1.2 and nothing else.

| Gate | Result |
| --- | --- |
| Live rustls (TLS 1.2 only) | Full handshakes plus data both ways for **30 combinations** of suite, curve and key type (ECDSA P-256, P-384, Ed25519 and RSA leaves), a 70 KB transfer each way, `close_notify` both ways, a TLS 1.3-only server (refused with its `protocol_version` alert surfaced), and a client-certificate request. |
| Live OpenSSL over a real socket | Nine tests against `openssl s_server`: all six suites (with OpenSSL reporting the one negotiated), all three curves, **all six RSA signature schemes** (PKCS#1 v1.5 and PSS at SHA-256/384/512, which rustls cannot be made to produce), ECDSA and Ed25519 certificates, an optional and a required client certificate, a TLS 1.3-only server, and a certificate for another name. Run by CI. |
| Independent signature vectors | 11 handshake signatures made by Python `cryptography` (OpenSSL): every scheme, plus the cross-curve ECDSA cases below. |
| Message codec | 14 tests. Each message parses and encodes as inverses; **every strict prefix and every trailing octet** of each is refused. |
| Refusals | A scripted server, built from this crate's own primitives and used only to make the client refuse, with a control test proving its correct flight completes under every framing (one message per record, all in one record, one octet per record, and a Finished split across records). 58 refusal and connection tests, each asserting the *specific* error: version, suite, compression, extended master secret, secure renegotiation, unsolicited extensions, point formats, certificate (empty, malformed, untrusted, wrong name, expired, wrong key type for the suite), key-exchange signature (tampered, wrong client random, wrong server random, wrong curve label, wrong key, unoffered curve or scheme, wrong key type, empty, invalid or low-order public key), message order, `ChangeCipherSpec` timing and body, `Finished` (wrong data, reflected, wrong key, plaintext, short, trailing message, no CCS), alerts, record framing and limits, and the established connection (renegotiation request, forged, replayed and reordered records, alerts, close). |
| Every byte | One bit flipped in **every byte** of a valid server transmission (more than 400 handshakes; the test fails if it runs fewer). The only flips that still complete are the minor-version byte of each unprotected record header, which RFC 5246 appendix E says is ignored. Stable across six runs. |
| Mutation | 34 deliberate bugs across the client, the message module and the verifier (skipping the chain check, the signature check, either random in the signed data, the CCS rules, the Finished check, a transcript message, the extension allowlist and more). All 34 caught. |
| Fuzz | New libFuzzer target `client12`: arbitrary record sequences into the handshake, which reaches the X.509 parser and path validator through `Certificate`. 1.95 million executions in 90 seconds, no findings; and it asserts nothing completes a handshake. |

**A real bug, found by a real peer, that rustls could not have found.** The first version of the TLS 1.2
verifier copied TLS 1.3's rule that an ECDSA scheme names a curve. OpenSSL 3.0 signs a **P-384** key
with the **SHA-256** scheme whenever the client lists it first, because in TLS 1.2 the scheme names only
the hash. The client refused a correct server. rustls never shows this, because its client lists the
P-384 scheme first. The fix reads the curve from the key; the vectors above pin it, and the doc comment
that wrongly said other stacks enforce the binding was corrected.

Two smaller things the live peers taught:

- For an ECDSA certificate, RFC 8422 §5.1 makes the **certificate's curve** part of what the client's
  `supported_groups` must cover, and OpenSSL enforces it. A client that offers only X25519 cannot talk
  to a P-256 ECDSA server. That is correct behaviour, and the interop tests now offer the right set.
- rcgen serialises Ed25519 keys as PKCS#8 v2, which OpenSSL 3.0's key reader refuses; that one test pair
  is generated by OpenSSL instead.

Decisions made, inherited by later stages:

- **Refuse, don't accommodate.** No extended master secret, no secure renegotiation, an unsolicited
  extension, or a `HelloRequest` mid-handshake (RFC 5246 would let a client ignore it) are all refused.
- **A `ChangeCipherSpec` is accepted at exactly one moment**, as one octet `0x01`, because accepting one
  early is CVE-2014-0224. (One further guard, that no handshake bytes are half-read at that moment, is
  unreachable because the state machine already refuses handshake records there; it stays as defence.)
- **The handshake buffer is capped at 128 KiB** so a server cannot make the client hold 16 MiB before it
  has authenticated anything.
- **Warning alerts other than `close_notify` are advisory** and ignored, as RFC 5246 allows; during the
  handshake `close_notify` is an error.
- **A renegotiation request after the handshake** is answered with a `no_renegotiation` warning and the
  connection carries on.

Not done in this stage, deliberately:

- **No downgrade protection.** This client offers only TLS 1.2, so there is nothing to downgrade from;
  the `DOWNGRD` sentinel is not checked. A client offering both versions must, and will (stage 4b-v).
- **No resumption** (neither session ids nor tickets), **no client certificate** (an empty one is sent
  when asked), **no ALPN**, **no OCSP stapling**. Each is a later stage or a seam concern.
- **Not wired to any seam type.** Nothing in `TlsStream`, `AsyncTlsStream` or the default build uses it.
- **The ECDSA scheme/curve leniency is TLS 1.2's, and applies only to TLS 1.2.** It must never be reached
  from a TLS 1.3 connection.
- **Leaf `keyUsage` (digitalSignature) is not checked**, as in the TLS 1.3 path.
- Secrets are not zeroized on drop, as elsewhere in the engine.

Limits of this evidence: the scripted server and the client share this crate's primitives, so the refusal
tests show the client rejects what the *script* got wrong, not that the script is a faithful TLS server
(the live rustls and OpenSSL tests are what show that). The fuzz, mutation and bit-flip figures are single
runs. Interop is with two implementations, both tested on loopback.

## Stage 4b-iv: done

`handrolled::server12` (`ServerConfig12`, `ServerHandshake12`), `ClientHello12` in `handshake12`, and a
role on `Connection12` so one established-connection type serves both ends. A sans-IO ECDHE server for
the same six AEAD suites, with an optional client certificate, speaking TLS 1.2 and nothing else.

| Gate | Result |
| --- | --- |
| Live rustls client (TLS 1.2 only) | Every suite and group the key type allows, each the only one offered so it is actually negotiated, for ECDSA P-256, P-384, Ed25519 and RSA keys; 40 KB each way; `close_notify` both ways; a TLS 1.3-only client refused with `protocol_version`; client authentication with every client key type, a missing certificate (refused when required, tolerated when not) and an unrelated CA. |
| Live OpenSSL `s_client` over a real socket | Eight tests, run by CI: all six suites with the negotiated one checked, each group, RSA-PSS at three hashes, Ed25519, a client certificate of three key types, a missing one, and a **renegotiation request** (`R`), which OpenSSL reports as refused with `no renegotiation`, i.e. our warning was exactly `no_renegotiation(100)`. OpenSSL also **verifies the chain and the name** of what the server sends. |
| Self-interop | This crate's client against this crate's server for every suite and its key type, 50 KB each way, `close_notify`, and a ClientHello delivered in 1, 2, 3, 5, 7 and 50-octet records. |
| Refusals | 41 tests asserting the specific error and alert: client version, `supported_versions` (including malformed lists), extended master secret, secure renegotiation in both forms, a stale `renegotiated_connection`, compression, suites with no overlap or the wrong key type, groups, signature schemes, point formats, duplicate extensions, over-long session ids, truncation of every prefix of a hello, an unbounded handshake message, and message order. |
| Client authentication | A certificate without a `CertificateVerify` (the impersonation case), a signature that does not verify, **someone else's trusted certificate with my signature**, a skipped `Certificate`, an empty one followed by a proof, malformed chains. Each against a control that passes. |
| Scripted client | Built from the public primitives only, so it can send what the real client never would: **a perfectly encrypted wrong `Finished`** (all 96 bit flips, the server's own label, a transcript without `ClientKeyExchange`, four wrong lengths), a handshake message or half a header trailing the `Finished` in its record, a `ChangeCipherSpec` with handshake bytes half-read, and a renegotiation request (and seven near misses). Its control completes and its verification of the server's `Finished` passes. |
| Every byte | One bit flipped in every byte of the client's flight (744 handshakes). Exactly the 16 flips of the two plaintext records' minor-version octet still complete; the test requires that count and no other. |
| Mutation | 46 deliberate bugs in `server12`, the hello parser and the server role of `Connection12`. 44 caught on the first or second round. The first round **survived 9**, and they were worth it: a wrong-but-valid `Finished` was never sent by any test, nothing put a message after `Finished`, the server's renegotiation recognition was untested, and one check turned out to be dead code (removed). One more was a mutation that changed nothing, and was replaced by a test that two handshakes never share a random or an ephemeral key. |
| Fuzz | New libFuzzer target `server12` (client authentication on or off by the first octet): 1.37 million executions in 90 seconds, no findings; asserts no input completes a handshake. |

**What the live peers taught, again.** For an ECDSA key the server must check that the client listed the
certificate's curve (RFC 8422 §5.1), and a rustls client restricted to X25519 fails that: the first test
run expected a handshake and the server was right to refuse. The matrix now asserts that refusal as its
own case. Nothing in the server was wrong on first contact with rustls or OpenSSL; the bugs were all in
tests, which is not evidence of correctness but is worth saying.

Decisions made in this stage:

- **The server's key decides what it can offer.** Suite authentication, signature scheme and certificate
  curve are all derived from the `SigningKey`; a P-256 key is never offered an RSA suite.
- **No PKCS#1 v1.5 signing.** `SigningKey` signs RSA-PSS only. A client that offers nothing else (an
  `openssl s_client -sigalgs RSA+SHA256`) is refused with `NoSharedSignatureScheme` rather than answered
  with something it did not offer. Adding PKCS#1 is a `SigningKey` change, not a server one.
- **`ServerError` is shared with the TLS 1.3 server** and gains seven variants. `Connection12` still
  reports `ClientError`, because it was written first; renaming it is a clean-up for 4b-v when the
  errors are unified.
- **A `CertificateVerify` signs the raw handshake messages** (TLS 1.2), checked with
  `verify_tls12_signature`, so the TLS 1.2 verifier's ECDSA leniency now has a second caller.

Not done in this stage, deliberately:

- **No downgrade protection** (stage 4b-v): this server speaks 1.2 only, so it writes no `DOWNGRD`
  sentinel. It also refuses a client that offers 1.3 only, instead of upgrading it.
- **No resumption, no ALPN, no SNI-based certificate selection**; the server presents one chain.
- **Not wired to any seam type** or the default build.
- Secrets are not zeroized on drop, and ECDHE shared secrets pass through a `Vec`, as in the client.

Limits of this evidence: the scripted client shares this crate's primitives, so its refusals show what
the server rejects of *its* mistakes; the live tests are what show the server is a faithful TLS server.
The fuzz, mutation and bit-flip figures are single runs. Interop is with two implementations on loopback.

## Stage 4b-v: done

`handrolled::negotiate`: `ClientConfigBoth` / `ClientHandshakeBoth`, `ServerConfigBoth` /
`ServerHandshakeBoth`, and `Established` (a 1.3 or 1.2 connection). One endpoint that speaks both
versions, built by choosing and then stepping aside: each side buffers records until the first
handshake message is whole, decides, and replays the buffered records into the existing 1.3 or 1.2
machine. Nothing in those machines changed behaviour for a caller that does not use this module.

What it adds, which is the property the track gave up by learning to speak something older:

- **A combined ClientHello.** `ClientHandshake::start_offering_tls12` builds one hello that offers
  1.3 and 1.2 (versions, both suite lists, the PKCS#1 schemes, extended master secret, secure
  renegotiation, point formats) *before* any PSK binder is computed, so resumption still binds the
  whole message, and a HelloRetryRequest's second hello offers exactly the same.
- **The sentinel, written.** A two-version server answering in 1.2 puts `DOWNGRD\x01` in
  `ServerHello.random` (`ServerHandshake12::downgraded`, used by this module only).
- **The sentinel, checked.** A client that offered both and is answered in 1.2 refuses a hello that
  carries it (`ClientError::DowngradeDetected`). A client that offered only 1.2 does not: it is a
  1.2-only client of a 1.3-capable server, told the truth, and checking would break it.
- **`TLS_FALLBACK_SCSV`** (RFC 7507): a hello whose best is 1.2 and that signals a fallback is
  refused with `inappropriate_fallback`.

| Gate | Result |
| --- | --- |
| Live rustls, both ends | Every pairing of both / 1.2-only / 1.3-only against our server and against our client picks the version rustls does, with data both ways. |
| Active attacker, rustls as the honest end | TLS 1.3 stripped from a hello in flight: our server answers in 1.2 **with the sentinel** and a real rustls client aborts naming the downgrade; our client, given a 1.2 reply carrying rustls's sentinel, returns `DowngradeDetected`. Controls complete in 1.3. Against a 1.2-only server (which cannot write one) the same attack is caught by the transcript at the `Finished`, and the test says so. |
| Live OpenSSL over a socket | Four tests, run by CI: OpenSSL as client at default, `-tls1_3` and `-tls1_2`; our client against `s_server` at the same three settings; a **real `-fallback_scsv`** hello, refused with the alert OpenSSL names; and a TCP proxy that strips 1.3 from our client's hello so the **sentinel OpenSSL writes** is what our client catches. Five consecutive runs clean. |
| Framing | A hello and a ServerHello delivered in 1, 2, 3, 7, 33 and 40-octet records, and a record that ends inside the *next* message, choose the right version every time. |
| Refusals | Neither version offered, client_version below 1.2, malformed version lists, a non-handshake or wrong first message, a first message that never ends (cut off at 128 KiB on both sides), an alert before the ServerHello (reported by name). |
| Hello content | The combined hello is parsed and checked field by field, and so is the plain 1.3 hello, which must say nothing about 1.2. |
| Mutation | 31 deliberate bugs (sentinel absent, at the front, wrong octet, written by the standalone server; checked too widely or not at all; each dispatch rule inverted; fallback ignored or too broad; framing and caps; hello contents; the retry forgetting the 1.2 offer). First round: 28 caught, 2 were redundant code (removed, the 1.2 server already refuses a hello with no usable version), and 1 was a real gap (the framing test did not assert *which* version was chosen). All genuine mutants now caught. |
| Fuzz | Two new libFuzzer targets, `negotiate_server` and `negotiate_client`, which also reach the TLS 1.3 server and client for the first time: 631k and 622k executions in 60 seconds each, no findings; each asserts nothing completes a handshake. |

**Found by the tests before this was committed.**

- *A bug in the first version of the choice.* It judged the version of a ServerHello with
  `messages()`, which errors on a partial trailing message; when a record boundary fell inside the next
  message the choice silently defaulted to 1.3, and a 1.2 server was refused with a confusing error.
  The framing test found it; the fix is to read only the complete prefix. A second, identical slip on
  the server side was hidden by a patch that did not apply and was found by the next test.
- *A missing API.* The TLS 1.3 `Connection` had no `close()` (the 1.2 one has), so an endpoint could
  not end a 1.3 conversation with `close_notify`. Added: one record, symmetric with 1.2, exercised by
  the OpenSSL tests (which report an unclean EOF without it).

**A correction to stage 4b-iv.** The clippy check I reported for that commit used a pattern that never
matched coloured output, so it could not fail. The pushed commit had one dead-code error in a test
(`Scripted::honest_finished`) that `-D warnings` rejects. It is fixed here; the checks in this stage
were run with a pattern shown to catch an error first.

Decisions made in this stage:

- **The version is chosen on the first whole message, once.** A client that lists 1.3 and has nothing
  else in common with this server fails; it is not retried in 1.2, because the 1.3 machine has
  already consumed the hello. rustls would fall back; here that would need the dispatcher to keep a
  second copy of the hello state, which is the complexity this design avoids.
- **Resumption is 1.3 only.** The PSK rides in the combined hello and a 1.2 reply ignores it.
- **`Established` is an enum, not a trait object.** The two connections differ in their incoming
  types, and flattening them would hide exactly the distinction a caller has to make (tickets exist in
  one and not the other).
- **No redundant version check in the dispatcher.** Whether a hello offers any usable version is the
  1.2 server's rule, stated once.

Not done in this stage, deliberately:

- **No ALPN, no SNI-based certificate selection, no 1.2 resumption.**
- **Not wired to any seam type** or the default build.
- **`Connection12` still reports `ClientError`**, and `Established` therefore exposes two error types.
  Unifying them is a breaking rename and belongs with a decision about the seam.
- Secrets are not zeroized on drop, as elsewhere in the engine.

Limits of this evidence: the MITM tests rewrite one message and show the sentinel and the transcript
catch it; they do not show a defence against an attacker who can also break a signature. The mutation,
fuzz and framing figures are single runs. Interop is with two implementations on loopback.

## Stage 4b-vi: done

`rusty_tls/bogo/` is a shim that lets BoringSSL's protocol suite (BoGo, Go) drive the engine as a
hostile peer. It is a separate unpublished package (like `fuzz/`), pinned to one BoringSSL commit, run
by `bogo/run.sh`, and run by CI. Result at the pinned commit: **490 passed, 0 failed, 7476 skipped**.
The skips are 4718 tests that need a shim flag it does not implement (client auth, resumption, 0-RTT,
ECH, ...), 2065 DTLS, 693 QUIC, and 139 listed by exact name in `bogo/config.json`, each with a reason.
Globs were tried first and abandoned: they hid 125 tests that pass.

BoGo's failures were findings, and there were many. Fixed in the engine, each with an in-repo test:

- **Alerts.** Every error now maps to the alert RFC 8446 / 5246 names (`handshake_alert`,
  `record_alert`, one table per role, pinned by `tests/handrolled_alerts.rs`), and the connection says
  it: plaintext before keys, under the handshake key (plus the compatibility CCS) on the client, under
  the application key on the server after its flight. Illegal warnings in 1.3 are `unexpected_message`;
  `user_canceled` is advisory and counted.
- **Key-change boundaries** (RFC 8446 §5.1): handshake data spanning or following a key change is
  `unexpected_message`, on both roles and in 1.2 before the flight is built.
- **HelloRetryRequest**: cookie echoed, `key_share` optional, an empty HRR refused, the compatibility
  CCS sent once, before the second hello.
- **Extension strictness**: unsolicited extensions in ServerHello / EncryptedExtensions / Certificate
  entries are `unsupported_extension`; `key_share` without `supported_groups` (and the reverse) is
  `missing_extension`; duplicate key shares; `supported_versions` beats `client_version`; a future
  `legacy_version` is tolerated by servers (`ClientHello::parse_lenient`; `parse` stays strict so the
  encode/parse round trip holds); session id over 32 octets; a 1.2 ServerHello that does not echo the
  session id; wrong-length 1.2 Finished; CertificateRequest CA list validated.
- **Record layer**: a TLS 1.3 inner plaintext over 2^14 + 1 octets is `record_overflow`.
- **Certificates**: a leaf with `keyUsage` lacking `digitalSignature` is refused (`bad_certificate`).
- **Resource limits** (`handrolled::limits`, BoringSSL's numbers, all *consecutive* counts that real data
  resets): 32 empty records (and 1.3 CCS), 4 warning alerts, 32 KeyUpdates. Applied in all six
  machines, including the dispatchers before a version is chosen.

| Gate | Result |
| --- | --- |
| BoGo | 490 pass, 0 fail at the pinned commit; CI fails on any FAIL or on fewer than 400 passes. |
| Mutation | 20 deliberate bugs in the new limits, boundaries, allowlists, alert paths and key-usage check. First round: 12 caught, 6 survived (CCS counting on both roles, the client's mid-handshake empty-record count, both 1.2 client counters, the key-usage check), plus 2 patterns that matched two places (replaced by 4 anchored ones, 22 mutants in all). Each survivor got a test; all 22 are now caught. |
| Fuzz | `client12`, `server12`, `negotiate_server`, `negotiate_client`, 45 s each: 943k, 812k, 640k, 536k executions, no findings. |
| Existing suites | rustls and OpenSSL interop unchanged. Several existing tests asserted the old behaviour (a fatal `user_canceled`, a removed `key_share` triggering a retry, a duplicate-extension alert, and others); each was changed on purpose with a comment citing the RFC or BoGo case. |

Deliberate differences from BoringSSL, all disabled by name with a reason: every KeyUpdate request is
answered (BoringSSL coalesces); a renegotiation request gets a `no_renegotiation` warning and the
connection carries on; 0-RTT is refused; RSA signing is PSS-only; the shim and the 1.2 client carry no
client identity.

Not done: tlsfuzzer (BoGo covered the same ground and was runnable); a cap on concurrent handshakes
(that belongs to whatever owns the listener, not to a sans-IO machine); a cost limit on a hostile
`Certificate` beyond the existing handshake-size cap. BoGo is one corpus at one commit, and 4718 of its
tests were never run against this engine because the shim cannot express them, so "0 failed" means "of
the 490 it could run".

## Stage 7: BoGo shim flags (first tranche)

Not part of the original six stages; added because stage 6 left most of BoGo unrun. Ranked by
`BOGO_UNIMPLEMENTED_LOG` (the shim records which flag stopped each test), the first tranche was the
cheap flags: single-version client and server scenarios (the shim now drives the standalone 1.3 and 1.2
machines, not only the dispatchers), `-curves` (a group the engine lacks is "unimplemented", never
dropped), `-async` (no callbacks exist to defer, so the outcome the runner checks is the same) and
`-check-close-notify`. Result: **590 passed, 0 failed** (was 490), 217 disabled by name.

BoGo found two engine bugs in the newly reachable tests, both fixed with a test that fails without the fix:

- **A server sent `handshake_failure` for an invalid key share** (point not on the curve, truncated,
  padded, compressed, a low-order X25519 point). RFC 8446 §4.2.8.2 says `illegal_parameter`; the client
  already did. `ServerError::Kx(BadPeerKey)` now maps to it; other key-exchange errors stay
  `handshake_failure`.
- **A client checked the cipher suite before the version.** A TLS 1.2 server names suites a 1.3 client
  never offered, so the client reported `illegal_parameter` for the suite instead of `protocol_version`
  for the version. The suite check now follows the version check, as the session-id check already did.

78 newly runnable tests fail by design and are disabled by exact name with a reason: TLS 1.0/1.1
scenarios, static-RSA suites, SHA-1 signatures, BoringSSL's leaf-curve policy (`CheckLeafCurve`), and its
choice of `0x0301` as the first record version (RFC 8446 §5.1 allows `0x0301` or `0x0303`).

What is left, from the log: credentials (`-expect-selected-credential`, `-new-x509-credential`), verify
callbacks (`-verify-fail`, `-expect-verify-result`), resumption (`-resume-count`, `-psk`), OCSP, ALPN
and client authentication. Resumption is the next best: the engine already issues and accepts 1.3
tickets, and `-resume-count` alone gates about 265 tests; the shim would need to repeat the connection
in one process.

## Stage 8: BoGo resumption

`-resume-count`, and BoGo's per-connection flags (`-on-initial-X` for the first connection,
`-on-resume-X` for later ones): the shim now runs every connection of a scenario in one process, keeps
the tickets a client was given, offers the oldest on the next connection, and gives a server a ticket key
that outlives a connection (`-no-ticket` turns tickets off). **645 passed, 0 failed** (was 590), 264
disabled by name.

BoGo found four TLS 1.3 server defects the in-repo resumption tests never reached, each now fixed with a
test that fails without it (mutation-checked):

- **A PSK offered without `psk_key_exchange_modes` was ignored**; RFC 8446 §4.2.9 says abort with
  `missing_extension`, whether or not the server would resume.
- **Tickets were issued to a client that could not redeem them** (offered no `psk_dhe_ke`).
- **A retried ClientHello could drop every PSK** the first one offered; it may drop unusable ones, not all
  (`missing_extension`).
- **Binder count mismatch was a `decode_error`**; with a non-empty binder list of the wrong length it is
  `illegal_parameter`, while an empty list stays a decode error.

47 newly runnable tests fail by design and are disabled with reasons: TLS 1.2 resumption (session IDs and
tickets; none exists), 1.2 without extended master secret, ECH, BoringSSL's GREASE ticket extension, and
TLS 1.0/1.1 variants.

Still unrun: credentials, verify callbacks, OCSP, ALPN and client authentication (see the log with
`BOGO_UNIMPLEMENTED_LOG`).

## Stage 9: BoGo client authentication

The shim now presents a client certificate (`-cert-file`/`-key-file`, TLS 1.3 only) and, as a server,
asks for one (`-verify-peer`, `-require-any-client-certificate`, verified against `-trust-cert`; the
engine has no accept-any-chain mode and does not get one for a test suite). **721 passed, 0 failed** (was
645), 321 disabled by name.

Two client defects found and fixed, each with a test that fails without it (mutation-checked):

- **A handshake CertificateRequest with a non-empty `certificate_request_context` was answered.** RFC 8446
  section 4.3.2 reserves a context for post-handshake requests; the client now refuses with
  `decode_error` (`ClientError::NonEmptyRequestContext`).
- **A CertificateRequest with an empty `signature_algorithms` list was read as "no scheme in common".**
  The list is `<2..2^16-2>`, so empty is malformed.

57 newly runnable tests are disabled with reasons: 22 TLS 1.0/1.1 variants; 28 TLS 1.2 client-auth
scenarios, because the TLS 1.2 client carries no identity; 6 where the server accepts only PKCS#1 v1.5 or
SHA-1 (this client sends an empty Certificate, which RFC 8446 section 4.4.2.3 allows, where BoringSSL
aborts); 1 BoringSSL certificate-type policy.

**The 1.2 client identity is the largest engine gap this surfaced.** A TLS 1.2 client cannot authenticate
to a server that requires a certificate, which a drop-in replacement for rustls has to do. It is a real
feature (Certificate, CertificateVerify over the 1.2 transcript, `ClientConfig12.identity`), not a shim
flag, and is the proposed next stage.

## Next action

Decision for the owner, not another stage: the engine now has the evidence the earlier stages could
produce (rustls and OpenSSL both directions, BoGo, mutation, fuzz, CI). Whether that meets the bar for
the seam is theirs to set. If it does, stage 5 of the track is wiring (ALPN, SNI certificate selection,
1.2 resumption are the known gaps) and an ADR superseding ADR-0002. If the bar includes more of BoGo,
the next increment is a TLS 1.2 client identity, then credentials (`-new-x509-credential`) and verify callbacks (`-verify-fail`).
