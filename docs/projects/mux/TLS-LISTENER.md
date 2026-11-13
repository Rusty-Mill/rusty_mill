# TLS listener (card #506): design note

Status: Accepted-pending-PR-confirmation. Nano answered the three questions (rusty_term DECISIONS D235-D237, 2026-11-13) but has not yet confirmed on PR #1656. Nothing is built. Owner gates apply before any code: a `rusty_tls` change,
a new dependency, or a new crate; each capability row needs its own go.

## Context

Remote access today is the SSH bootstrap (2026-10-29 row in [DECISIONS.md](DECISIONS.md)):
the user's SSH login is the identity. The TLS listener was set aside then because no
first-party X.509 generator exists and `rcgen` is dev-only in `rusty_tls`. Tailscale identity
(C27) was named next. A listener is the common prerequisite: C27 has no peer to identify
until the server accepts TCP.

## Threat model

- **Attacker on the network** reads, replays, alters or impersonates the server: TLS 1.3 with
  a pinned server certificate answers it.
- **Attacker who finds the port** guesses a credential: a 256-bit random token, no passwords,
  a per-peer failure delay, and a closed connection after the first bad preface.
- **A stolen token** is the weak point; rotation and per-token revocation limit it. A token
  maps to a principal, so it carries exactly that principal's power (account-level, as in
  shared mode, D232).
- Out of scope: a hostile local root, and traffic analysis.

## Proposed design

1. **Two ways in, one listener type.** `conn::run` is made generic over a read and write half
   (about ten lines, SPIKE-4-ROAMING); Unix, TCP+TLS and a tailnet TCP listener share it.
2. **Certificate: the user supplies it.** `--tls-cert` and `--tls-key` name PEM files only
   the server's account can read, checked as the login map's files are. No generator is
   built, so no new dependency and no `rusty_tls` change. A user can make one with
   `openssl` or `tailscale cert`. If Nano wants a generator, that is a separate owner gate.
3. **Pinning, not a CA.** The client stores the server certificate's SHA-256 fingerprint per
   host (in `hosts.json`, C35) on first use and refuses any change until the user re-pins.
   It uses the existing `rusty_tls` trust hooks if they allow a fingerprint check; if not,
   that is the one `rusty_tls` change to ask for.
4. **Token.** `rusty-mux-ctl token create NAME` prints a random 256-bit token once; the
   server stores only its SHA-256 and a name in a root-or-account-only file, with the
   principal it maps to. The client sends it as the first frame after the TLS handshake,
   before the existing preface. `token rotate NAME` replaces it, `token revoke NAME` removes
   it; a reload closes only that token's connections.
5. **Principal mapping.** A token principal is `token:NAME`. It reaches an account through
   the login map (C26) and the shared-mode allowlist the same way `uid:N` does, so there is
   one access table, not two.
6. **Tailnet without TLS (C27).** On a tailnet address the WireGuard layer already
   encrypts and authenticates the peer. The server asks `tailscaled` for
   `/localapi/v0/whois` of the peer address and maps the tailnet user or tagged device to a
   principal (`ts:LOGIN` or `ts-node:NAME`). No token or certificate is needed there.
   Binding only to the tailnet address is mandatory for this mode.

## Order

1. Generic `conn::run` (no behaviour change).
2. Tailnet listener with whois (C27): the smallest useful step, no TLS.
3. TLS listener with user-supplied certificate, pinning and tokens (#506).

## Questions for Nano

All three answered "Recommend" (2026-11-13, project thread cmsg_014f6o52E2iAAAgMm9v8hh1w5paA1SoRFaJMyA9U2SJRr4); pending his confirmation on the PR.

1. Build the tailnet path (C27) first and the TLS listener after? **Answered: yes** (D235).
2. For TLS, the user supplies the certificate and the client pins its fingerprint (no generator)? **Answered: yes** (D236).
3. Tokens stored hashed, shown once, rotated by `rusty-mux-ctl`? **Answered: yes** (D237).
