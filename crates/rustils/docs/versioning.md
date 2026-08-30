# Versioning — the rustils workspace and the wider ecosystem

Three separate questions, answered separately: how versions move
*inside* this repo (§1), the exact rule for what bumps what (§2), and
how *other* repos should depend on this one, and this one on them,
including what "combining" versions across repos actually means (§3).
Getting them tangled is what makes a versioning policy unreadable.

## 1. Inside this workspace: three groups, not one, not six

Every crate here already shares one version number
(`version.workspace = true`, `[workspace.package].version` in the root
`Cargo.toml`) — lockstep by construction today. That's right for some
of these crates and wrong for others, so this policy narrows it to
three groups instead of flattening to either extreme (one version for
everything, or six independent ones):

- **The PAL group — `platform`, `platform-linux`, `platform-windows`,
  `platform-mock`, (since rustils#48) `platform-bsd`, and (since the
  rustils#88 follow-up) the dev-only `platform-parity` — stays
  lockstep**, one shared version. These crates change together in
  practice, not in theory: every Net slice this phase touched all the
  then-existing members in the same PR (the trait, both real backends,
  and the mock), because a trait method that exists on `platform` and
  not on one backend doesn't compile. `platform-bsd` joined the group
  (as `platform-macos`; renamed in rustils#86 when its cfg gate widened
  past Darwin) at whatever version was already current — its own arrival
  changed no existing crate's public item shape, so it did not itself
  force a bump — rather than starting independently, for the identical
  reason: `platform-bsd 0.9.0` means nothing without knowing which
  `platform` 0.9.0 it implements. Independent per-crate SemVer here
  would mean bookkeeping five version numbers that must already move
  in lockstep to stay buildable — busywork with no compatibility
  signal behind it.
- **`winargv` versions independently.** It already has its own
  lifecycle distinct from the PAL's convergence churn: the extraction
  map's own "handback" plan (D3) has it flowing back to rush/
  rusty_win32 as a standalone artifact, and it's functionally
  complete and fuzz-hardened already — its version should track *its
  own* changes (a new escaping edge case, a fuzz-found fix), not get
  bumped every time `platform` grows a new Net slice underneath it.
- **`coreutils` versions independently**, for close to the opposite
  reason: it's a reference-consumer proving `platform`'s API, not
  itself depended on by anything outside this repo. Bumping it in
  lockstep with `platform` would force a `coreutils` release on every
  PAL change even though nothing about `coreutils` itself moved.

Mechanically: give `winargv` and `coreutils` their own `version = "…"`
field (dropping `version.workspace = true`), while `platform`/
`platform-linux`/`platform-windows`/`platform-mock`/`platform-bsd`/`platform-parity`
keep sharing `[workspace.package].version`.

`publish = false` stays as-is; crates.io publication is a separate,
later decision, not implied by anything in this document.

## 2. The exact numbering rule while everything is 0.x

SemVer's `0.y.z` leaves the industry's actual convention ambiguous —
some crates treat `y` as "breaking only," others bump `y` for any
public-API change at all. This document picks one, explicitly, rather
than leaving it to individual judgment call every time:

> **At `0.y.z`: any change to the public API surface — additive or
> breaking — bumps `y`. `z` is reserved for changes that touch no
> public item's shape at all** (a bug fix in an existing function's
> body, a doc correction, an internal refactor, a test-only change).

This collapses the usual 1.0+ distinction between "MINOR: safe to
auto-upgrade" and "MAJOR: review needed" into one question at 0.x:
*did any `pub` item's shape change?* If yes, `y` bumps and a `^0.y.z`
consumer won't silently pick it up — which is also just what Cargo's
own caret-requirement resolution already does for `0.x` deps, so this
rule is really "stop treating additive changes as free, because Cargo
doesn't either."

**Why not the gentler reading** ("adding a method is backward
compatible, so it's patch-level")? Because `platform`'s public items
are almost all `pub trait`s, and a trait is consumed on *two* sides —
callers and implementers. `TcpStream::set_read_timeout` was additive
for every existing *caller* of `TcpStream`, but breaking for a
hypothetical fifth backend implementing the trait: it wouldn't compile
until it added the new method. A crate whose public surface is mostly
traits doesn't get to assume "additive" means "safe" the way a
struct-and-functions crate can — so this document doesn't try to
distinguish the two cases and instead treats any shape change as
`y`-worthy, full stop.

### Worked example: this phase's own history, renumbered

Concretely, if this rule had been in force from `platform` 0.1.0
onward:

| Change (already landed) | Bump | Why |
|---|---|---|
| TCP slice (`Net`, `TcpStream`, `TcpListener`) | 0.1.0 → 0.2.0 | new public traits |
| Unix sockets slice (`UnixStream`, `UnixListener`, `unix_connect`/`unix_listen`) | 0.2.0 → 0.3.0 | new public traits + `Net` gained methods |
| Unix sockets stale-cleanup-bind fix (the contract correction before merge) | *no bump* | fixed before the 0.3.0 tag existed at all — nothing shipped the wrong contract under a version number |
| UDP datagram slice (`UdpSocket`, `Net::udp_bind`) | 0.3.0 → 0.4.0 | new public trait + `Net` gained a method |
| Unix-socket parity suite | *no bump* | test-only, no `pub` item changed |
| `TcpStream::set_read_timeout` | 0.4.0 → 0.5.0 | existing public trait gained a required method — breaking for implementers even though additive for callers, per the rule above |

Six real changes, three of which actually moved the number. That's the
rule doing its job: it's supposed to be quiet exactly when nothing
public moved.

### 1.0 stays out of scope

Moving to `1.0` is itself a real decision, not a default that happens
on a schedule, and shouldn't happen while surfaces are still growing
capability in response to their first real consumer (as Net just did,
three times this phase alone). Revisit once Security/Terminal/PTY/
Windowing have landed and gone through the same kind of post-"done"
correction Net did — if they *don't* need one, that's the actual
signal `1.0` might be close.

## 3. Across the ecosystem: pinning, and what "combining" means

The concrete gap this document exists to close: `rusty_rdp`'s new
`platform` dependency (the `platform` feature, landed this phase) is
an **unpinned** git dependency tracking `main` — every future
`rusty_rdp` build resolves whatever is newest on `platform` at fetch
time, with no signal about whether that's still compatible. This
isn't a new problem needing a new pattern — it's this repo's own
existing convention (`platform-linux`'s `track-p` dependency on
`rusty_libc` is already `rev = "dfa4e8c1f…"`, a full pinned commit, not
a branch) not having been applied symmetrically the one time the
dependency direction reversed: rustils pins what it depends on, but
until now nothing enforced that what depends on rustils does the same.

**The rule, in both directions**: a git dependency on a sibling repo
in this ecosystem pins `rev` to a specific commit (a tag's commit is
fine and more readable than a bare SHA; a bare SHA is fine too — same
guarantee either way) — never a branch name, never left unpinned.

- **When to cut a tag**: consumer-gated, the same judgment call the
  RFC's own §3 makes for building surfaces at all — tag when an
  external consumer (a sibling repo's convergence PR) actually needs
  one to pin against, not speculatively on every merge. A PR in
  `rusty_rdp` (or `shh`, or `rusty_tail`) that adds or bumps a
  `platform` dependency is the trigger; cutting the tag is part of
  that PR's own prerequisite work, the same way `rusty_libc`/
  `rusty_win32` version bumps already get called out explicitly in
  this repo's own landed-notes when Track P or extraction work depends
  on them.
- **Bumping an existing pin**: a deliberate, reviewable diff (one line
  in the dependent's `Cargo.toml`), not something that happens by
  `cargo update` silently picking up whatever's newest — which an
  unpinned branch dependency would otherwise do on every fresh
  `Cargo.lock`.
- **Follow-up owed**: `rusty_rdp`'s `platform` dependency (its own
  `Cargo.toml`, landed in `baileyrd/rusty_rdp#30`) should move from
  unpinned-`main` to a real pinned `rev`/tag as its own small
  follow-up PR in that repo — this document doesn't do that itself,
  it just names the debt.

### There is no ecosystem-wide version number, deliberately

The natural next question — "so what version is *the ecosystem* at?"
— has a deliberate non-answer: there isn't one, and there shouldn't
be. rusty_regx, rusty_whisper, and rusty_lines have nothing to do with
`platform`'s version and would gain nothing from being forced onto a
shared number with it; the RFC's whole consumer-gate philosophy (§3)
already rejects speculative coupling between repos that don't need
each other yet, and an umbrella ecosystem version would be exactly
that kind of coupling, just expressed as a number instead of code.

**What "combining" actually means, mechanically**: each consumer's own
`Cargo.lock` *is* the combination — the full, exact set of pinned
`rev`s (or crates.io versions, once that's a thing) it happens to
depend on, computed independently per consumer. `rusty_rdp` pinning
`platform` at commit X and `shh` pinning it at commit Y is not a
conflict to reconcile; they're two different consumers on two
different schedules, exactly as independent as their own repos are.
Nothing here tries to make every consumer agree on one `platform`
commit at once.

**The one place real coupling *does* need to propagate**: when a
sibling repo's own version bump changes what the PAL can observably
do — e.g. a `rusty_libc` release adds a syscall wrapper `platform-linux`
then wires up under `track-p` — that wiring PR bumps the PAL group's
own `y` too (§2's rule: `platform-linux`'s public behavior changed,
even though `platform`'s own trait didn't). The `rusty_libc` pin bump
and the PAL's own version bump land in the *same* PR, so there's never
a state where `platform-linux`'s version claims more (or less)
capability than its pinned `rusty_libc` actually has.

## 4. Mechanics: when to bump, when to tag, and tracking what changed

- **Bump the version number in the same PR that changes the public
  API** — not deferred to a later "release PR." `Cargo.toml`'s version
  field should never be stale relative to what's actually on `main`;
  a PR that adds a `pub` item and doesn't bump `[workspace.package]
  .version` (or the independent `winargv`/`coreutils` version) is
  incomplete, the same way a PR that changes behavior without updating
  `docs/behavior/net.md` is incomplete.
- **Tag lazily, at the consumer-trigger point §3 already describes** —
  bumping the number and cutting a tag are two different moments on
  purpose. Most PRs bump the number and stop there; a tag only gets
  cut when an external consumer is actually about to pin against that
  state. This avoids a graveyard of tags nobody ever references, while
  guaranteeing a tag exists by the time anything needs one.
- **`CHANGELOG.md` records every version bump** in the PAL group and in
  `winargv`/`coreutils` independently — one entry per `y` (or `z`)
  bump, named for the PR/feature that caused it, in the style the
  worked example in §2 already previews. This is the practical answer
  to "what changed at 0.4.0 vs 0.5.0" that skimming git log across a
  dozen PRs doesn't give a consumer for free.

## What this document doesn't decide

- crates.io publication timing for any crate here (`publish = false`
  stands until that's a deliberate, separate decision)
- a version-bump policy for the *other* repos in the ecosystem
  (rusty_libc, rusty_win32, rusty_rdp, etc.) beyond "pin to what you
  depend on here" — each of those repos' own maintainers own their own
  release discipline; this document only governs rustils' side of the
  relationship
- the `1.0` transition itself, beyond naming it as a real decision
  that hasn't been made yet
