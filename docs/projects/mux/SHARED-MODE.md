# Shared mode (C26): design note

Status: Proposed. The first step of card #274 (Arc 1, go from Nano 2026-11-12). Nothing here
is built. The three open questions at the end are Nano's to answer on the PR.

## What it is

Shared mode is **one server account serving several principals**. C26 names it "one server,
several accounts, where C33 must be restricted". A process has one uid, so the "several
accounts" part is login mode: each account gets its own server. Shared mode is the other case,
where people share one account's server and its blocks.

Today a personal server admits only its own uid (`peer_check`, `server/src/lib.rs`), and every
client carries that one principal. A second principal reaches a server only through login
mode's forwarded preface. Shared mode adds a second way in: the server admits listed peers
itself, and each connection carries its own kernel-verified `Principal` (`uid:N` and the name).

## Built already, reused unchanged

| Need | Where |
|---|---|
| Principal on every client, never client-claimed | C25, D8 |
| Takeover only by the same principal; a leaked token starts a client of its own | `takes_over`, 2026-11-01 row |
| Force-disconnect restricted: server account, own principal, or the session's creator | #1590, D161, D164, D166 |
| Ownership per client, orphan-close right to the owner's principal | C46, D85 |
| The only cross-principal input mechanism: GrantLease, RevokeLease, RequestLease | C77, bits 36/37, D120, D140 |
| A UDP association belongs to its opener's principal | 2026-11-01 row |

## Proposed access model

1. **Admission.** The server reads an allowlist (see question 1). A peer not on it is closed at
   once. The allowlist starts empty, and root is admitted only by a rule naming it, never by a
   wildcard.
2. **Power of an admitted principal.** Blocks run as the server's account, so admitting a
   principal grants account-level power, as a login-map entry does. The note says so in the
   server's help and in the user docs.
3. **Rename and close** of another principal's session, window or block use the C33 rule
   (`may_disconnect`): the server account, the target's own principal, or the session's
   creator. C46 owner refusals come first. Recorded as D231, a default.
4. **Tokens, ownership, UDP:** no change (table above). Shared mode adds nothing for UDP on a
   socket server.
5. **No new wire.** Admission is server-side. Questions 2 and 3 may add a feature bit; they
   say so.

## Children of #274

| Child | Depends on | Gate |
|---|---|---|
| Shared mode (this note) | #507, done as #1590 | Nano's answers below; independent security review |
| PAM account and session checks in login mode | none | Owner gate: libpam is a native dependency (link or dlopen is part of the approval); security review (the front door is root) |
| UDP for account servers behind the front door | none | Security review; a preface change visible to old peers would be a wire change |

C14 live sharing (#278) builds on shared mode and C77 leases and adds no second lease system.

## Questions for Nano

1. **Admission.** Who may connect to a shared server, and how is it configured?
   a) An allowlist file of `uid:N` or `user:NAME` lines, reusing the login-map rules and root
   protections, read at start and on SIGHUP. b) A Unix group on the socket. c) Only through the
   login front door, so no direct shared mode. Recommend a): it reuses the map code and fails
   closed when empty. Security-relevant.
2. **Visibility.** What does each principal see in `list`?
   a) Everything, with the C33 and rename rules as the only limits. b) Its own sessions plus the
   server account's. c) A per-session ACL. Recommend a) for the first slice, since admitted
   principals are on an allowlist; C14 adds per-session sharing later.
3. **Typing into another principal's block.** May a principal take the input lease of an open
   block it did not create?
   a) Yes, as today, with the lease holder's block size as the visible cost. b) A guest's
   blocks default to owner-only input, and the creator or server account grants through C77.
   c) The lease stays inside the creator's principal, and everyone else needs a C77 grant.
   Recommend c): the least new machinery, and the only option that stops a guest typing into
   the server account's shell without a grant.

**Attach (Nano, 2026-11-13, as relayed):** owner's blocks only. Guests list everything but attach only to blocks their principal owns or was granted (C77); the server account's shell is not attachable by guests.

**Tailnet (C27 slice 2, proposed):** `--tailnet ADDR:PORT` adds a listener on a tailnet address. A peer is served only if the allowlist names its `ts:LOGIN` or `ts-node:NAME`, with the same guest rules as above.

**Tailnet in login mode (C27 slice 3, proposed):** `--login MAP --tailnet ADDR:PORT` does the same on the front door, with the map in place of the allowlist: a `ts:LOGIN` or `ts-node:NAME` rule names the account the peer uses. No `=` and no `*` for tailnet peers.

A `ts:` or `ts-node:` entry matches the principal id exactly, so `ts:Nano@example.com` and `ts:nano@example.com` are different entries.
