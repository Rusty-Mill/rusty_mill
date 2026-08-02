# Design discussion — TLS and the net surface (rustils#70)

Not a decision record. This is the RFC-level research the owner asked for on
rustils#70's question — "when `rusty_request` (via `rusty_tokio`) eventually
needs HTTPS, what is the intended seam, and does anything belong in rustils?" —
written *before* any consumer passes §3's gate, on the same posture as
`docs/design-discussion-sandbox.md`: surface what the consumers, the operating
systems, and the ecosystem's own precedents actually offer, and the real open
questions that follow, for the owner to decide. Per §3, this document produces
rows and gate conditions, not code.

**Amended 2026-07-21**, in two ways. First, the original draft's honesty note
— that donor claims were grounded in this repo's records rather than fresh
source verification — is now superseded: a source-level survey of shh,
rusty_tail, rusty_rdp, rusty_provider, rusty_request, and rusty_tokio was
performed and its findings are folded in below (see *Prior art* and open
question 5's resolution; rusty_llama remains unverified and is still flagged
where it matters). Second, review of the merged first draft found three
technical claims about trust-store access that were wrong or understated —
the macOS anchor-enumeration API, Windows's lazily-populated root store, and
the inability of an anchor list to express distrust — all corrected in place
below, and all cutting the same direction: B1 is *best-effort* anchor
loading, materially less faithful than OS verification (B2) on Windows and
macOS.

## Context: the standing stance, restated precisely

The stance is not "rustils has no opinion on TLS." It is three specific,
already-recorded decisions:

1. **D16 (extraction map): no TLS obligation in `Net`.** All four named net
   consumers bring or inject their own wire crypto; `Net`/`TcpStream` carry no
   TLS concept, and `net.md` lists TLS/crypto under *Deliberately unspecified*
   — a designed exclusion, not a gap.
2. **The injection precedent (rusty_rdp).** rdp's `tls.rs` layers optional,
   injected rustls over code generic in `Read + Write` — TLS wraps the plain
   stream *above* the trait boundary.
3. **The narrow-extraction precedent (Csprng).** The one crypto-adjacent thing
   that *did* land here came from that same `tls.rs`: five hand-rolled
   `/dev/urandom` reads retired by `Csprng::fill_random` — one method, no key
   derivation, no algorithm choice, because that was all the named consumer
   needed.

So the research question is not "should rustils do TLS" in the abstract — it is
*which parts of the HTTPS-client problem, if any, are Layer-1/Layer-2 material
under this architecture, and which are permanently Layer-3*.

## What an HTTPS client actually needs

The minimum a real `https://` client must do, annotated by what kind of
problem each row is. This decomposition is the whole document in miniature —
every option below is a claim about where these rows live.

| # | Need | Kind of problem |
|---|---|---|
| 1 | TLS handshake + record protocol (1.2/1.3: key schedule, X25519/P-256 key exchange, AEAD record encryption, finished-message verification) | Protocol cryptography |
| 2 | X.509 chain building and validation (ASN.1/DER parsing, signature verification up the chain, expiry, name constraints, EKU) | Protocol cryptography + PKI logic |
| 3 | **Trust anchors** — the set of root certificates the chain must terminate in | **OS personality** — every OS keeps these somewhere different, in a different shape |
| 4 | Hostname verification (SAN/dNSName matching, wildcard rules) | Protocol logic (RFC 6125) |
| 5 | SNI (required by effectively every CDN-hosted server) | Protocol logic |
| 6 | ALPN (only if HTTP/2 ever; `http/1.1`-only clients can skip it) | Protocol logic |
| 7 | Session resumption / tickets | Protocol logic, optional (performance only) |
| 8 | Revocation (OCSP/CRL) | PKI logic, realistically optional for a non-browser client |
| 9 | A plain, nonblocking-capable byte stream underneath | **Already solved** — D16's boundary |

Rows 1–2 and 4–8 are the TLS library. Row 9 is done. Row 3 is the one row that
is *genuinely* about the operating system's personality — and notably, it was
never actually excluded by D16, which excluded wire-crypto behavior from the
*trait*, not trust-material access from the *repo*.

## What each OS actually offers (the Layer 0/1 survey)

This is the load-bearing section, because the three platforms are radically
asymmetric, and every design option inherits that asymmetry.

### Windows — TLS is a first-class OS service

- **A full TLS engine: SChannel via SSPI.** `AcquireCredentialsHandle` /
  `InitializeSecurityContext` / `EncryptMessage` / `DecryptMessage`. Notably
  for this repo's tastes, the SSPI handshake is *sans-IO*: the caller pumps
  opaque token buffers between the API and its own socket, so it layers over
  a nonblocking `TcpStream` (or `rusty_tokio`'s) without owning the I/O. All
  reachable from the existing `windows-sys` floor.
- **A full chain engine: CryptoAPI.** `CertGetCertificateChain` +
  `CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL, ...)` performs
  complete validation — chain building against the ROOT store, revocation if
  asked, hostname matching — as one OS call.
- **Trust anchors as an OS object — with a lazy-population trap.**
  `CertOpenSystemStoreW(L"ROOT")` + `CertEnumCertificatesInStore` yields the
  roots as DER blobs, honoring enterprise group-policy-deployed roots —
  something a bundled cert file can never do. But the ROOT store is *lazily
  populated*: Windows ships most roots via AuthRoot auto-update and fetches
  them on demand *during chain building*, so enumeration returns only what is
  currently cached. An anchor list built this way can fail validation for a
  site whose perfectly-trusted root simply hasn't been fetched yet; only the
  chain engine (`CertGetCertificateChain`) triggers the on-demand fetch. This
  is a documented limitation of the enumeration approach, and it bounds what
  B1 below can honestly promise on Windows.

### macOS — half a TLS service, a whole trust service

- **Trust evaluation is first-class and current:** `SecTrustCreateWithCertificates`
  + `SecTrustEvaluateWithError` with an SSL policy (`SecPolicyCreateSSL`)
  validates a chain including hostname, against the keychain's trust settings
  (again including MDM/enterprise-deployed roots). Anchor *enumeration* is
  where the first draft of this document was wrong:
  `SecTrustCopyAnchorCertificates` returns only Apple's *built-in* system
  roots — it does **not** include user- or MDM/admin-added anchors.
  Enumerating *effective* trust means walking the trust-settings domains via
  `SecTrustSettingsCopyCertificates` and interpreting per-certificate trust
  settings (including partial-trust and distrust records) — famously messy,
  and exactly why Apple steers callers toward evaluation rather than
  enumeration. Evaluation (B2's shape) is the first-class citizen here;
  enumeration (B1's shape) is a best-effort reconstruction.
- **The TLS engine story is worse:** Secure Transport (custom-IO callbacks, so
  it *can* layer over an external stream) is deprecated since 10.15; its
  replacement, Network.framework, owns the connection down to the socket —
  it does not layer over a stream someone else owns, which is exactly the
  wrong shape for wrapping `rusty_tokio::io::TcpStream`.
- Repo-local caveat: `platform-bsd` is deliberately net-only and thin.
  Anything here means linking Security.framework — a whole new framework
  admission for that backend, not a marginal addition.

### Linux — no OS TLS engine, no OS verifier, anchors are just files

- **The kernel's only TLS concept is kTLS** (`setsockopt(TCP_ULP, "tls")` +
  `SOL_TLS` key material): record-layer encrypt/decrypt offload *after* a
  userspace handshake has already derived the keys. It is a throughput/
  `sendfile(2)` feature, not a TLS implementation — there is nothing to
  hand a handshake to.
- **There is no OS chain verifier.** No syscall, no stable system service.
  OpenSSL/GnuTLS are distro *libraries*, not kernel ABI — building on them is
  the libc-tier question Track P exists to escape, but worse (soname churn,
  distro feature drift), and it would be a hard dependency where libc is at
  least universal.
- **Trust anchors are plain files with no single home:**
  `/etc/ssl/certs/ca-certificates.crt` (Debian),
  `/etc/pki/tls/certs/ca-bundle.crt` (RHEL/Fedora),
  `/etc/ssl/cert.pem` (Alpine), plus the `SSL_CERT_FILE`/`SSL_CERT_DIR`
  conventions. Reading them is `Fs` work; *knowing where to look* is the
  entire per-distro personality.

**The central finding:** "let the OS do it" is fully true on Windows, true
only for trust evaluation on macOS, and false on Linux — the platform
`rusty_tokio`/`rusty_request` actually develop on. Any design that leans on
an OS TLS engine is `Unsupported` exactly where the named consumer lives.

The wider Rust ecosystem independently converged on this same reading, which
is worth taking as confirmation rather than as a dependency plan (learn from,
don't depend — the §5.3 cap-std posture): `rustls-native-certs` exists solely
to do row 3 per-OS (schannel ROOT store / Security.framework / file-path
probing on Linux), and `rustls-platform-verifier` exists solely to do row 2
via the OS on Windows/macOS *with a webpki fallback on Linux* — because on
Linux there is nothing to delegate to.

## Prior art inside the ecosystem (source-verified, 2026-07-21 amendment)

Everything below except the rusty_llama row was verified against the actual
repos, per the sandbox discussion's standard.

- **rusty_rdp** (`src/tls.rs`, feature `tls`): the injection seam works in
  production — rustls optional, injected, the RDP-over-TLS protocol logic
  staying in a dependency-free core generic over the stream. Two facts the
  extraction map never recorded, both load-bearing here:
  1. `connect_tls` **performs no certificate verification at all** — a
     deliberate, documented `danger::ServerCertVerifier` that accepts every
     chain, because RDP servers overwhelmingly present self-signed
     certificates and rely on out-of-band trust; callers who want real
     verification are pointed at building their own rustls stream and using
     `new_enhanced`. So rdp loads **no trust anchors today** — it is not a
     B1 call site, and the ecosystem's one shipped TLS path is knowingly
     MITM-unprotected. If rdp ever grows real verification, *it* becomes a
     trust-anchor consumer too.
  2. Its module docs independently state this document's Option-D
     conclusion, verbatim: "A TLS stack is the one piece that cannot be
     hand-rolled responsibly." And it already consumes rustils in this exact
     path — `connect_tls_with_csprng` threads `platform::security::Csprng`
     into the CredSSP exchange behind an optional `platform` feature.
  Separately, rdp *does* hand-roll a large set of protocol-mandated legacy
  primitives (MD4/MD5/SHA-1/RC4/RSA/AES/HMAC/PBKDF2, `src/crypto/`) under an
  explicit "not safe for new designs" warning — obsolete algorithms the RDP
  wire format forces, not a counterexample to the TLS stance.
- **shh and rusty_tail hand-roll their *protocols*, not their primitives** —
  a correction to this document's first draft (and to the extraction map's
  shorthand). shh hand-rolls the SSH protocol (packet sealing, hybrid
  ML-KEM-768+X25519 kex, KDF, OpenSSH-compatible Ed25519 user certificates)
  but takes its primitives from dalek/RustCrypto crates (`x25519-dalek`,
  `ed25519-dalek`, `chacha20`, `aes-gcm`, `sha2`); rusty_tail's data plane is
  `boringtun`/`crypto_box` crates, and its control plane is Noise over
  **plain HTTP** against Headscale, with HTTPS explicitly "not yet
  supported" and its DERP client http-only. Even the hand-rolled-ethos repos
  drew the line *above* the crypto primitives — which strengthens, not
  weakens, the argument below against hand-rolling TLS. It also makes
  rusty_tail a **latent second HTTPS consumer**: real Tailscale control and
  DERP both run over TLS, so targeting production infrastructure eventually
  hits the same client-TLS wall as rusty_request.
- **rusty_provider** (not part of D16's survey, and not part of the
  hand-rolled family — a pragmatic tokio/reqwest stack): does real TLS three
  ways today — reqwest with `rustls-tls-native-roots` for provider APIs,
  `tokio-postgres-rustls` for the database, and a hand-built
  `rustls::ClientConfig` (`crates/router/src/persistence.rs`) that calls
  `rustls_native_certs::load_native_certs()`, warns per failed cert, and
  fails closed on zero roots. **The B1 operation already runs live in this
  ecosystem** — served by the ecosystem crate, with no expressed gap. That
  is CredentialStore-shaped evidence (complete, working, no desire to
  migrate), and it reshapes B1's gate condition below.
- **rusty_request and rusty_tokio**: verified exactly as rustils#70
  described — rusty_request rejects `https://` as a tracked gap (its proxy
  module defers CONNECT tunneling "until HTTPS lands"; even its jitter RNG's
  docs invoke "same reasoning as the TLS gap"), and rusty_tokio contains no
  TLS concept anywhere.
- **rusty_llama's optional server** is recorded in D16 as one of the four
  bring-your-own-crypto consumers; it was **not** part of the 2026-07-21
  survey, so this remains the one donor claim still resting on this repo's
  records rather than source.

The hand-rollability argument itself is unchanged and now better-grounded:
SSH-style and Noise-style protocols have no X.509/PKI, no ASN.1, no
version-and-ciphersuite negotiation matrix, and (in these tools'
deployments) pinned or TOFU keys. TLS + WebPKI is a different order of
attack surface, and its characteristic failures are *silent* — a validator
that accepts a bad chain passes every happy-path test ever written for it.
The parity regime has no oracle for "rejects what an attacker sends";
contrast Track P, where a wrong syscall wrapper fails loudly under the
existing suite. M1's own rule — hand-rolled only counts when it's *correct*
— is unusually hard to satisfy here and unusually expensive to check.

## The options

### Option A — status quo: the consumer injects a TLS layer (issue #70's option 1)

`rusty_request` (or `rusty_tokio` as a middleware layer) wraps the plain
stream in an async TLS implementation built or vendored outside this repo,
exactly as rdp does synchronously.

**Seam check — is the boundary already sufficient?** What a sans-IO TLS layer
needs from the stream underneath: read/write with honest `WouldBlock`
(`rusty_tokio`'s reactor path — raw-fd/raw-socket access + `set_nonblocking`,
landed as rustils#41/#48/#59 for Linux/macOS/Windows precisely for this
consumer), or blocking read with `set_read_timeout` (landed 2026-07-20 at the start of the rdp convergence, forced
by rdp). Nothing else — TLS needs no socket options `Net` doesn't already
expose. **The seam is complete today; Option A costs rustils zero code.**

The one real cost lands on `rusty_request`: taking rustls (or vendoring an
async TLS layer) cuts against its hand-rolled ethos. That is a Layer-3 values
question this repo's architecture deliberately leaves to the consumer — same
as it left shh's and rusty_tail's crypto choices to them.

### Option B — a narrow trust-material primitive (issue #70's option 2, split honestly)

The issue's "certificate validation / trust-store access via the OS" is
**two different primitives wearing one label**, and the platform survey above
splits them cleanly:

- **B1 — trust-anchor access: "give me the OS's root certificates," as raw
  DER blobs.** Implementable on all three backends
  (CertEnumCertificatesInStore / trust-settings enumeration on macOS /
  distro-path probing + `SSL_CERT_FILE` on Linux). No cryptography enters
  rustils — the consumer's TLS layer does all validation; rustils only
  answers the genuinely OS-personality question of *where the roots live and
  how to read them*. Naturally one method; naturally mockable (a mock
  backend with configurable anchors gives TLS-using consumers hermetic
  tests, the same service `platform-mock` provides everywhere else).
  **This is the Csprng-shaped candidate — but its honest contract is
  *best-effort* anchor loading**, with three fidelity limits the first draft
  understated (2026-07-21 amendment):
  1. *Windows misses uncached roots* — the ROOT store is lazily populated
     via AuthRoot (see the platform survey above), so enumeration can omit a
     trusted root the chain engine would have fetched on demand, producing
     false validation failures.
  2. *macOS enumeration is a reconstruction, not an API* — the one-call
     anchor API returns built-in roots only; effective trust requires
     walking trust-settings domains and interpreting per-cert settings.
  3. *A DER list cannot express distrust* — OS stores carry negative and
     partial-trust records (explicitly distrusted certs, constrained roots);
     `Vec<CertDer>` has no way to say "never accept this one," so a consumer
     validating against B1's output can accept a chain the OS itself would
     reject.
  These are the documented limits of the rustls-native-certs approach
  industry-wide, and they are simultaneously the strongest argument *for*
  B2 on the platforms that have it — recorded here so the B1-vs-B2 trade is
  weighed on accurate facts, not the first draft's tilt.
- **B2 — OS chain *verification*: "validate this presented chain for this
  hostname."** A real one-call OS API on Windows and macOS; **no OS API at
  all on Linux.** A Linux backend would have to hand-roll X.509 path
  validation — ASN.1 parsing plus RSA/ECDSA signature verification — which is
  precisely the crypto this repo refuses, smuggled in through a "narrow"
  door. So B2 is honest only as an *asymmetric, platform-optional* surface
  (`Unsupported` on Linux — mechanically precedented by Tun-on-Windows and
  Sandbox-on-Linux, but those served consumers who lived on the supported
  platform; here the named consumer develops on the unsupported one). B2
  should never be the primary path, only a possible later upgrade for a
  consumer that specifically wants OS-policy semantics on Windows/macOS —
  the exact posture rustls-platform-verifier takes.

Gate condition to record for B1, revised by the 2026-07-21 survey: *a
**hand-rolled-family** consumer (rusty_request once HTTPS lands, rusty_tail's
control plane going TLS, or rusty_rdp growing real verification) needs OS
trust anchors **without** taking `rustls-native-certs`, and is observed
hand-rolling anchor loading — distro path probing, cert-store reading — at
real call sites.* That is the same duplication signal that gated Csprng, and
it cannot exist before Option A happens first. The family qualifier is new
and matters: rusty_provider already performs this exact operation in
production via the ecosystem crate, contentedly — which is CredentialStore-
shaped evidence (a complete, working implementation with no live gap is not
a forcing consumer), not Csprng-shaped evidence. rusty_provider's
`persistence.rs` config builder is, however, the best in-ecosystem design
reference for what B1's semantics should be if it ever gates in: per-cert
error tolerance with warnings, fail-closed on zero roots.

### Option C — an OS-native TLS engine surface (`SChannel`/Secure Transport behind a trait)

Genuinely M1-attractive on Windows — the SSPI token pump is a first-rate NT
lesson, and its sans-IO shape fits this repo's boundary instincts. But:

- **No Linux backend can exist** without pulling in a TLS library, so the
  named consumer's own platform is permanently `Unsupported` — the surface
  would serve a consumer that hasn't been named instead of the one that has.
- Secure Transport is deprecated; its successor can't wrap an external stream.
- The surface is irreducibly wide (protocol versions, ALPN, resumption,
  alert/error taxonomy, renegotiation) — the opposite of the one-method
  discipline every admitted primitive here has followed.
- The parity regime's textually-identical-suites convention has no good story
  for a domain where one backend is structurally absent rather than divergent
  on lines.

**Recorded as researched and not recommended.** Revisit only if a
Windows-only (or Windows-first) consumer names itself with a concrete need —
at which point the SSPI half stands on its own merits.

### Option D — hand-rolled TLS inside rustils

Concrete scope, so the size is on record: a TLS 1.3 client alone means the
handshake state machine, the key schedule (HKDF), X25519, at least one AEAD
(AES-GCM or ChaCha20-Poly1305), RSA-PSS + ECDSA signature verification,
X.509/ASN.1 DER parsing, SAN/hostname matching, and the record + alert
layers — rows 1, 2, 4, 5 of the needs table, every one security-critical,
most failing silently when wrong (see the shh/rusty_tail contrast above).
There is no parity oracle for "resists an attacker," and no OS ground truth
to diverge-registry against. This is the one option both M1 and M2 argue
*against* rather than trading off: M1 because correctness is unverifiable at
this repo's tooling level, M2 because a foundation must not carry a
hand-rolled validator other projects then trust.

**Not recommended in rustils, at any layer.** If the owner ever wants the
learning (it is real — M1 doesn't stop being curious because the artifact is
dangerous), the shape that fits the architecture is a Layer-3 toy *beside*
the PAL — a `rusty_tls` in the rusty_lines/rusty_regx position — explicitly
never wired into anything that trusts its verdicts.

### Option E — kTLS, for completeness

Linux-only record-layer offload after a userspace handshake; useful when a
consumer wants `sendfile(2)`-over-TLS throughput (static file serving), i.e.
no consumer in this ecosystem today and plausibly ever. Pure kernel-ABI
material, so it *would* be Layer-1-shaped if a consumer appeared. Row only;
nothing to design.

## The core tension: "TLS support" is three problems wearing one label

Mirroring the sandbox discussion's finding that "sandbox" named two unlike
things:

1. **The protocol/crypto engine** (needs 1–2, 4–8) — OS personality on
   Windows, deprecated-or-wrong-shaped on macOS, nonexistent on Linux.
   Permanently Layer 3 under this architecture (Options A/D).
2. **Trust-material access** (need 3) — OS personality on *every* platform,
   just asymmetrically shaped (store API vs. framework call vs. file
   probing). The only Layer-2 candidate, and D16 never actually excluded it
   (Option B1).
3. **The wire seam** (need 9) — already built, already proven by the
   #41/#48/#59 sequence that this exact consumer forced.

The standing stance "no TLS in rustils" is precisely "no (1) in rustils."
Keeping that stance and eventually landing B1 are fully compatible.

## Recommendation (research input, not the decision)

- **Now:** Option A, which requires nothing — the seam is verified complete
  above. `rusty_request` stays `http://`-only or injects TLS at Layer 3 when
  ready; rustils changes nothing.
- **Pre-identified gated slice:** B1 (`load OS trust anchors as DER blobs`),
  fourth Security slice alongside Csprng/Sandbox, unparked only on the
  revised gate condition above (hand-rolled-family consumer, no
  `rustls-native-certs`), and specified as *best-effort* per its documented
  fidelity limits. Raw DER at the boundary — no ASN.1 types in rustils, the
  byte-oriented §5.2 instinct applied to certificates; parsing stays
  consumer-side; rusty_provider's config builder is the semantics reference
  (warn per bad cert, fail closed on zero).
- **Recorded as researched-and-declined:** C (no backend on the consumer's
  platform), D (unverifiable correctness in a trust-critical artifact), E
  (no consumer, trivially revisitable).
- **B2** noted as a possible much-later Windows/macOS-only upgrade behind
  the same primitive's door, never the primary path.

## Open questions for the owner, not decided here

1. **Does B1 wait for its gate, or is a Sandbox-style speculative build
   acceptable?** The Sandbox precedent shows the owner may explicitly accept
   speculative-build risk when a validated donor design exists. The
   2026-07-21 survey sharpened this question's facts in both directions:
   there *is* now a validated in-ecosystem design to mirror
   (rusty_provider's `build_rustls_client_config`), but its very existence
   — complete, working, no expressed desire to migrate — is the exact
   CredentialStore signal that argued for holding. And no hand-rolled-family
   consumer loads anchors at all yet (rdp verified as loading none — it
   skips verification entirely). The evidence now leans toward waiting.
2. **Which domain does B1 land in if/when gated — `security` (beside Csprng,
   as trust *material*) or `net`?** Recommendation embedded above: `security`,
   which also keeps `net.md`'s "Deliberately unspecified — any TLS/crypto"
   line true forever.
3. **Linux anchor-source policy for B1:** probe the known distro paths only,
   honor `SSL_CERT_FILE`/`SSL_CERT_DIR` first, or both — and what does
   `platform-mock` assert about ordering/precedence? Note the two on-disk
   layouts: a single bundle file *and* a directory of hashed symlinks
   (`/etc/ssl/certs` on Debian is both at once) — a probing policy has to
   decide which wins and whether directories are enumerated. (This is where
   most of the real Linux design content lives; and per the amended platform
   survey above, Windows/macOS are *not* the "single API call" the first
   draft claimed — enumeration has real fidelity limits on both.)
4. **Who is the consumer for gating purposes — `rusty_request` or
   `rusty_tokio`?** The TLS layer could live in either (request-level vs. a
   runtime-provided stream wrapper). It changes nothing about rustils' seam,
   but §3's table wants a *name*, and the anchor-loading call sites will live
   wherever the TLS layer does.
5. **~~Is fresh donor verification needed before any of this advances?~~
   Resolved 2026-07-21** — the survey ran against shh, rusty_tail, rusty_rdp,
   rusty_provider, rusty_request, and rusty_tokio at source level; findings
   are folded into the *Prior art* section above. The headline answers: rdp
   loads **no** native anchors (it skips verification entirely, by
   documented design), so rusty_request remains B1's first hand-rolled-family
   call-site candidate; rusty_provider — outside the original survey — turns
   out to already perform B1's operation via `rustls-native-certs`,
   reshaping the gate condition; and rusty_tail's plain-HTTP control plane
   is a latent second HTTPS consumer. The one residue: rusty_llama was not
   surveyed, and its optional server's TLS shape still rests on D16's record
   rather than source.

## What this document does not decide

Whether B1 ever gets built, on what timeline, or in what exact shape — and
nothing here weakens §3: `rusty_request` has no HTTPS code path today, so no
gate is passed and no code follows from this document. This is the input to
the owner's call that rustils#70 asked to have on record, not the call
itself.
