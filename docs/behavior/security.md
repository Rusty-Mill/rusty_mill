# Behavior Spec — security (Csprng, CredentialStore, Sandbox)

The parity suite asserts the `Csprng`/`CredentialStore`/`TrustAnchors`
spec against every backend. Its assertion sets live once, in
`crates/platform-parity`; each backend's `tests/security_parity.rs`
records only which of them apply to it plus its own OS-specific
expectations, and `crates/platform-mock/tests/parity_conformance.rs`
runs them against the mock. `Sandbox` has no shared set — its whole
contract is a `SandboxStatus` that legitimately differs per backend and
per host kernel, so there is no cross-backend behavior to assert. `Sandbox`'s real Linux enforcement is
exercised separately, in
`crates/platform-linux/tests/security_sandbox.rs` — see that file's own
doc comment for why (confinement is irreversible for the calling
thread, so it needs subprocess isolation the shared parity-suite binary
can't give it). A backend that cannot honor a line gets a numbered
entry in `../divergences.md` citing the OS limitation — never
implementation convenience.

## Scope (all four Security surface slices)

RFC v2 R5+, decision D15.

**`Csprng`** (first slice): rusty_rdp is the forcing consumer — five
hand-rolled `/dev/urandom` reads across `src/krb5/kdc.rs` and `src/tls.rs`
for Kerberos nonces/confounders and CredSSP exchange material.
`Csprng::fill_random` retired all five with one narrow primitive — one
method, no key derivation, no algorithm choice, because that's all the
named consumer needed.

Backends draw from the OS CSPRNG directly rather than opening
`/dev/urandom` as a file: Linux uses the `getrandom(2)` syscall,
Windows `BCryptGenRandom` with the system preferred RNG. Avoiding a
file descriptor is deliberate — a caller running under `Sandbox`
confinement might otherwise have the `open` denied.

**`CredentialStore`** (second slice, Phase 6 item 2, rustils#76/#77/#78):
also **no confirmed live consumer** — nexus's own `CredentialVault` is a
complete, working wrapper over the third-party `keyring-rs` crate with
no gap and no expressed desire to migrate, so this landed as the same
kind of owner's-explicit-call the sandbox slice did, not a named-
consumer unparking. `get`/`set`/`available` only, matching the
roadmap's own documented scope — no `delete` (rustils#76's own scope
note), no key derivation, no attribute schema beyond `service`/
`account`. Split across three PRs given its size: #76 (trait, Windows
Credential Manager, mock, the `NullCredentialStore` disabled-mode
escape hatch — landed), #77 (a hand-rolled D-Bus client transport for
Linux — no existing D-Bus dependency, matching this repo's raw-bindings
philosophy — landed as `platform_linux::sys::dbus`, an internal
prerequisite with no `CredentialStore` behavior wired up yet; see that
module's own doc comment for the transport contract), #78 (the Secret
Service protocol (`org.freedesktop.secrets`) on top of #77, wired into
the real Linux implementation — landed as
`platform_linux::sys::secret_service`, with `LinuxCredentialStore` now
delegating to it in place of the #76 stub).

**`Sandbox`** (third slice, Phase 6 item 3): unlike `Csprng`, this
slice has **no confirmed live consumer** — it mirrors nexus's
`os_sandbox.rs` (the closest thing this repo has to a validated design
for this capability) as the owner's explicit call, not because a named
consumer asked for it. See `docs/design-discussion-sandbox.md` for the
full design discussion and its open questions. `Sandbox::confine_filesystem` narrows filesystem access
via Landlock (ABI v1, raw syscalls — no libc wrapper exists);
`Sandbox::block_inet_sockets` denies new `AF_INET`/`AF_INET6`/
`AF_PACKET` sockets via a hand-written seccomp-BPF filter (`x86_64`
only for now). Both mirror nexus's own two-independently-degradable-
calls shape exactly, not an invented combined API.

## Specified

- `Csprng::fill_random(buf)` fills `buf` entirely with random bytes, or
  returns an `Err` — it never returns `Ok` having written fewer bytes
  than `buf.len()`.
- Two consecutive calls to `fill_random` on the same instance do not
  return identical bytes. This is the one property every named
  consumer relies on (a nonce or confounder that repeats is broken);
  the parity suite asserts exactly this, not any specific distribution
  or entropy-quality claim.
- An empty `buf` (`fill_random(&mut [])`) is a no-op, not an error.
- A request larger than a single underlying syscall/API call reliably
  fills (Linux's `getrandom(2)` can return short for requests over 256
  bytes; the backend retries until `buf` is full).
- `Interrupted` from the underlying syscall (`EINTR`) is retried
  internally, never surfaced to the caller.

### `CredentialStore`

- `available()` reports `Available` (a real, reachable backing store),
  `Unavailable` (a real mechanism exists on this OS but isn't reachable
  right now — no forcing case in this slice's Windows implementation,
  since Credential Manager is a core OS service with no "not running"
  state; Linux's Secret Service, rustils#78, is where this value has a
  real forcing case), or `Unsupported` (no mechanism at all — every
  backend's stub state before its real implementation lands, and
  `NullCredentialStore`'s permanent state).
- `get(service, account)` returns `Ok(Some(secret))` for a stored value,
  `Ok(None)` for a clean miss — never an `Err` for "nothing stored under
  that name." `set(service, account, secret)` stores it, replacing any
  existing value under the exact same `(service, account)` pair;
  different `account`s under the same `service` are independent and
  don't collide.
- No `delete` — not part of the roadmap's documented scope for this
  slice; a caller that wants to remove a stored secret isn't served by
  this trait yet.
- `NullCredentialStore` (`platform::security`, portable, no `unsafe`):
  the disabled-mode escape hatch — `available()` is always
  `Unsupported`, `get` is always `Ok(None)`, `set` is accepted and
  silently discarded. A caller opts into this explicitly by constructing
  it; it is not itself a mechanism for auto-detecting "keyring
  integration disabled."
- Windows (`WindowsCredentialStore`, rustils#76): real, live-verified
  Credential Manager (`CredWriteW`/`CredReadW`, `CRED_TYPE_GENERIC`,
  `CRED_PERSIST_LOCAL_MACHINE`). Credential Manager's identity key for a
  stored credential is `(TargetName, Type)` alone — `UserName` is a
  display field, not part of it — so this backend composes `TargetName`
  from *both* `service` and `account` (`\u{1}`-separated) rather than
  `service` alone, so that two different `account`s under the same
  `service` land as two distinct Credential Manager entries instead of
  silently clobbering each other.
- Linux (`LinuxCredentialStore`, rustils#78): the Secret Service API
  (`org.freedesktop.secrets`) over `sys::dbus`'s hand-rolled transport
  (rustils#77) — `platform_linux::sys::secret_service`. Stateless: every
  call opens a fresh D-Bus connection and Secret Service session rather
  than holding one open, mirroring how the Windows backend also makes a
  fresh `CredWriteW`/`CredReadW` call each time. `available()` reports
  `Unavailable` for every reachability failure (no session bus, no
  Secret Service provider registered, no default collection, or a
  locked collection this non-interactive backend has no window handle
  to unlock via `Prompt`) — this is where `Unavailable` gets its first
  real forcing case in this slice, unlike Windows. Unlike the #76 stub
  this replaced, an unreachable store is a real `Err` from `get`/`set`,
  not a silent `Ok(None)`/`Ok(())` — a clean miss ("nothing stored
  under this name") and "the store isn't reachable right now" are
  different claims, and only the caller checking `available()` first
  opts into treating them the same way. Item identity is the
  `service`/`account` attribute pair (Secret Service's own
  attribute-dictionary addressing), matching the trait's contract
  directly with no `TargetName`-style composition needed. Live-verified
  against a real `dbus-daemon --session` + `gnome-keyring-daemon`
  pair spawned as a CI test fixture, the same live-verification bar
  #77's transport was held to.
- `platform-mock`'s `MockCredentialStore`: a faithful in-memory fake
  (unlike `MockSandbox`) — a get/set secret store genuinely can be
  faked without lying about a security property, unlike kernel-level
  process confinement.

### `Sandbox::confine_filesystem`

- Denies all filesystem access except read+execute under
  `readable_roots` and read+write+create+delete under `writable_roots`,
  for the calling thread only. Irreversible for that thread — no
  "widen back" call exists, matching Landlock itself.
- Returns `SandboxStatus::Enforced` when the kernel actually applied
  the ruleset, `NotEnforced` when Landlock is missing/too old/disabled
  (the call did not error, but nothing is confined — the caller must
  check this value), `Unsupported` on every non-Linux backend and on
  the mock (see below).
- A `readable_roots`/`writable_roots` path that doesn't exist, or that
  the calling thread can't open, is a real `Err` (`NotFound`/
  `PermissionDenied`) — never silently dropped from the ruleset, which
  would create a false sense of confinement for a root the caller
  thought was included.

### `Sandbox::block_inet_sockets`

- Denies opening new `AF_INET`/`AF_INET6`/`AF_PACKET` sockets from the
  calling thread onward. Already-open sockets are unaffected — this is
  "no new raw-internet sockets," not a kill switch on existing
  connections. `AF_UNIX` and every other syscall are untouched.
  Live-verified: after enforcement, `TcpListener::bind`/`UdpSocket::bind`
  fail with `PermissionDenied` (`EPERM`) while
  `UnixListener::bind` keeps working.
- Returns the same three-way `SandboxStatus` `confine_filesystem` does.
  On Linux, `Enforced` is expected on any real host — seccomp filter
  mode has existed since Linux 3.5 — so `NotEnforced` here would be
  unusual (only genuinely ancient or seccomp-disabled kernels).
- `x86_64` only for now: the filter's mandatory architecture check
  (the standard seccomp-BPF defense against a 32-bit syscall-number
  reinterpretation bypass) is `AUDIT_ARCH_X86_64`-specific. Every other
  architecture reports `Unsupported` rather than installing a filter
  that assumes the wrong arch.

## `TrustAnchors` (fourth slice, rustils#88)

`rusty_tls` is the forcing consumer: it dropped `rustls-native-certs` to
hand-roll trust-anchor loading, and rather than grow three per-OS
backends in a crate that had none, that mechanism lands here. This is the
"B1" slice `docs/design-discussion-tls.md` researched and parked, gated
in on 2026-08-02 — see that document's *B1 reopened* section for why it
is here and why the TLS protocol itself never will be.

**One method, raw DER out.** `load_anchors() -> Result<Vec<Vec<u8>>>`.
This surface answers only *where does this OS keep its root certificates
and how do I read the bytes*. It never parses, validates, or verifies:
no chain building, no signature checks, no ASN.1. Certificates cross the
boundary as bytes for the same reason every other byte-oriented surface
here does (RFC v2 §5.2), and every interpretation of them belongs to the
consumer's TLS layer.

### Guaranteed on every backend

1. **Per-anchor errors are tolerated, not reported.** An entry that
   can't be read or decoded is skipped. Real trust stores routinely
   carry a few damaged entries, and one must not cost the caller the
   other several hundred.
2. **Zero usable anchors is an error, never `Ok(vec![])`.**
   `ErrorKind::NotFound`. A caller handed an empty set would trust
   nothing and fail every connection with a confusing per-connection
   error; failing at the source names the real problem.
3. **Stateless.** Every call re-reads. A machine whose trust store just
   changed does not keep serving the old set for the life of the
   process.

### Per-backend mechanism

| Backend | Mechanism |
|---|---|
| `platform-linux` | `SSL_CERT_FILE`, else `SSL_CERT_DIR`, else first existing distro bundle file, else first existing distro cert directory — first match wins *exclusively*, never a union |
| `platform-bsd` (macOS) | `SecTrustCopyAnchorCertificates` (Security.framework) |
| `platform-bsd` (other BSDs) | The same file probing as Linux, with BSD paths |
| `platform-windows` | `CertOpenSystemStoreW("ROOT")` + `CertEnumCertificatesInStore` |
| `platform-mock` | Exactly the anchors the test pinned; `empty()` stages the fail-closed path |

Bundle-before-directory is what lets one Linux policy cover every distro
without special-casing: RHEL ships only a bundle, Debian ships both a
bundle and a directory of hashed symlinks pointing at the same
certificates. Preferring the bundle reads one file instead of several
hundred symlinks, and never double-loads.

### Best-effort — three fidelity limits, on every platform

The anchor set is **not** a faithful reproduction of what the OS itself
would trust, and no implementation of this trait can make it one. These
are documented rather than papered over, and are the same limits
`rustls-native-certs` carries industry-wide:

1. **Windows' ROOT store is lazily populated** via AuthRoot, so
   enumeration can miss a root the OS chain engine would have fetched on
   demand — producing a validation failure the OS itself would not have.
2. **macOS anchor enumeration is a reconstruction, not an API.** The
   one-call form returns built-in roots only; effective trust needs
   walking per-domain trust settings. Doing so is a strictly larger
   surface, deliberately left as follow-up rather than smuggled in.
3. **A flat list cannot express distrust.** OS stores carry explicit
   distrust and constrained-root records; `Vec<Vec<u8>>` has no way to
   say "never accept this one," so a consumer validating against this
   output can accept a chain the OS itself would reject.

A consumer needing true OS-policy semantics wants the platform's own
chain *verification* API instead — option B2 in
`../design-discussion-tls.md`, deliberately not offered here because
Linux has no counterpart to it at all, and a Linux backend would mean
hand-rolling X.509 path validation: precisely the cryptography this
workspace refuses, through a door labelled "narrow."

## Deliberately unspecified

- Which anchors `load_anchors` returns, or how many — that is whatever
  the host machine trusts, so the parity suite asserts the *shape* of
  the answer (non-empty, DER-tagged, stable across two calls) and never
  a particular anchor set. Pinning one would test the CI image rather
  than the backend.
- Whether `load_anchors` reflects OS trust *policy* — it reflects the
  anchor list, which is a different and strictly weaker claim (see the
  three fidelity limits above).
- Any statistical randomness-quality property (entropy estimate,
  distribution uniformity, resistance to specific cryptanalytic
  attacks) — this trait asserts behavior (fills the buffer, doesn't
  repeat), not cryptographic strength, which is the underlying OS
  CSPRNG's responsibility, not this trait's to re-verify.
- Blocking behavior before the OS CSPRNG is seeded at early boot — in
  practice this is over well before any consumer here runs (a
  Kerberos client authenticating against a KDC is never on the boot
  critical path), so this trait makes no promise about the pre-seed
  window and no backend implementation goes out of its way to avoid
  blocking there.
- `platform-mock`'s `MockCsprng` byte *values* — deterministic and
  reproducible across test runs by design (a fixed seed), but not
  meant to resemble real randomness statistically; only "fills the
  buffer" and "doesn't repeat between calls" are asserted, matching
  every other backend.
- `platform-mock`'s `Sandbox` — there is no in-memory equivalent of
  kernel-level process confinement to fake, unlike `MockNet`/`MockDir`
  faking a socket or filesystem. `MockSandbox` always reports
  `Unsupported`, the same honest answer a real backend with no
  confinement mechanism gives; a mock that claimed `Enforced` here
  would be lying about a security property.
- macOS confinement — no backend exists (no donor; Seatbelt/
  `sandbox_init` and the App Sandbox entitlement system don't obviously
  fit an arbitrary, non-packaged CLI process — see
  `docs/design-discussion-sandbox.md` question 4).
- Windows confinement — no backend exists for the same reason
  (restricted tokens/AppContainer assume a packaged-app identity).
- Non-`x86_64` `block_inet_sockets` — reports `Unsupported`
  categorically rather than attempting a filter without the matching
  architecture check.
- shh's privilege-separation pattern (fork + socketpair + credential
  drop to protect a secret) — a different problem from confinement,
  doesn't fit this trait or `platform::process`'s current shape, and
  stays entirely out of scope here (see
  `docs/design-discussion-sandbox.md`).
- `CredentialStore` deletion, key rotation, credential attributes/labels
  beyond `service`/`account`, and multi-collection support (Secret
  Service's own concept, once rustils#78 lands) — none of these are part
  of the roadmap's documented scope; a future addition if a real need
  appears, not freelanced here.
- Exact wire compatibility with every possible Secret Service provider
  implementation (rustils#78) — only the `org.freedesktop.secrets`
  interface's documented contract is targeted, not any particular
  provider's (`gnome-keyring`, `kwallet`, etc.) implementation quirks.
- Windows `WindowsCredentialStore`'s `TargetName` composition scheme
  (the `\u{1}`-separated encoding) — an internal representation detail,
  not a promised wire format; a caller that inspects Credential Manager
  directly (e.g. via `cmdkey`) sees this encoding, but nothing in this
  trait's contract promises it stays this exact shape.
