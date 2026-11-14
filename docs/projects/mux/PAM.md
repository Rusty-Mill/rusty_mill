# PAM account checks for login mode (card #1460): design note

Status: Proposed. Nano gave the go ("PAM", 2026-11-14) and chose to load libpam with `dlopen`
(2026-11-14, decision card; rusty_term D266, D267). He has not confirmed the design below on
the PR, and Dottius's security review is still owed. Nothing is built. Owner gates apply
before code: this is a new native dependency (approved as `dlopen`, see 3) and no new crate.

## Context

Login mode (C26) is a root front door that starts and relays to per-account servers. It
admits a principal if the map names it, so an expired, locked or time-limited account is
still served (RISKS R17: "No PAM yet"). The site's PAM policy is the place that decides
whether an account may log in. Reading `/etc/shadow` ourselves would ignore it.

## Scope

- **Account checks only**: `pam_acct_mgmt` on the target account (expired, locked, time and
  host rules). Never `pam_authenticate`, never passwords: the map and the transport already
  authenticate the peer (REX-DELTA R8).
- Every principal source, since the check is on the account the map names: local uid, SSH
  user, tailnet principal (C27).
- **No `pam_open_session`.** Account servers outlive the front door, so no process could
  close the session and the front door's would leak. Deferred.

## Design

1. **Off by default.** `--pam[=SERVICE]` on `--login`. Service `rusty-mux`, file
   `/etc/pam.d/rusty-mux`. A sample ships under `docs/`; nothing installs it.
2. **Loading.** `dlopen` of `libpam.so.0` once at start, by absolute path from a short list
   of system directories (`/lib/x86_64-linux-gnu`, `/usr/lib64`, `/usr/lib`, and their
   siblings) with `RTLD_NOW|RTLD_LOCAL`, so the root process never searches
   `LD_LIBRARY_PATH` (D267). The functions are the five symbols used
   (`pam_start`, `pam_acct_mgmt`, `pam_set_item`, `pam_end`, `pam_strerror`) resolved by
   `dlsym` into a small `Pam` struct of typed function pointers. Needs `dlopen`, which the
   workspace `libc` dependency already exposes: no `-dev` package at build, no new crate,
   no new Rust dependency, and a host without libpam still runs the binary.
3. **Fail closed, at start.** With `--pam` the front door refuses to start if libpam does
   not load, if the service file is absent (Linux-PAM would fall back to `other`, which may
   allow), or if a probe `pam_start` fails. Only `/etc/pam.d/rusty-mux` counts: a file in a vendor
   directory (`/usr/lib/pam.d`) or an entry in `/etc/pam.conf` is not accepted, so a site
   that relies on one is refused at start, never silently allowed. Without `--pam` nothing is loaded.
4. **Where.** In `admit`, after the account lookup and the `still_by` re-check, before
   `server_for`. The call runs on the blocking pool under a timeout (5 s; `pam_ldap` can
   hang). A timeout leaves its thread running, so at most 16 checks run at once and a
   17th is refused, as for whois lookups.
5. **Call.** `pam_start(service, account, conv, &h)`; a conversation function that answers
   every prompt `PAM_CONV_ERR`; `PAM_RUSER` set to the peer's local user, and **unset for a tailnet peer**: its `user` is
   empty (C27), and a `ts:` id is not a local name that `pam_access` could match; `pam_acct_mgmt(h,
   PAM_SILENT)`; `pam_end`. Result `PAM_SUCCESS` admits. Everything else refuses,
   including `PAM_NEW_AUTHTOK_REQD` (an expired password cannot be changed here).
6. **A refusal looks like "no rule".** The front door closes the socket and logs the PAM
   reason to stderr. Nothing about account state reaches the client: no wire change and no
   account-state oracle.
7. **Honest help text.** `--pam` help says it checks the account, not the person.

## Threat model (RISKS R17)

- A flaw in the check must not widen admission: PAM can only add a refusal, and every
  error path refuses.
- libpam runs in the root front door with its modules (site-chosen). A buggy module can
  hang (timeout, cap) or crash the front door; the account servers survive (C26) and the
  supervisor restarts it. This is the cost of using the site's policy.
- The probe at start catches a missing library or service before any peer connects.
- A tampered `/etc/pam.d` needs root: out of scope, as for the map.

## Tests

- Fake libpam (a test `.so` or an injected `Pam`): success admits; each error, a timeout
  and `PAM_NEW_AUTHTOK_REQD` refuse; the 17th concurrent check is refused.
- Start refuses with `--pam` when the library, the service file or the probe fails.
- Without `--pam`, behaviour is unchanged (existing `tests/login.rs`).
- A manual root check against a real `pam_nologin`/`pam_time` service, recorded in
  `docs/projects/mux/`.

## Rollback

Drop `--pam`: no state, no wire change, no file written.

## Gates and order

1. This note: Nano confirms on the PR.
2. Build PR (one): `Pam` loader, `--pam`, the check in `admit`, tests, R17 updated. Needs
   Dottius's security review before queueing.
