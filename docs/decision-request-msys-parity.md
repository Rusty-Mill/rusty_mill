# Decision needed: msys-parity — proceed (and with which subsystem first), or park it?

Not a design document. Every document in this family ends the same
way: *no named consumer, gated on an owner call.* This one exists to
make that call easy to actually make — a one-page summary plus an
explicit ask, not new research.

## The five documents this decision covers

- `docs/design-discussion-msys-parity.md` — the original four-subsystem
  survey, grounded in `msys2-runtime`'s real source.
- `docs/design-discussion-msys-pgid-table.md` — subsystem 1, shared
  pgid/session table.
- `docs/design-discussion-msys-signals.md` — subsystem 2, arbitrary
  signal delivery.
- `docs/design-discussion-msys-stop-cont.md` — subsystem 3, real
  Stop/Cont.
- `docs/design-discussion-msys-pty.md` — subsystem 4, pty line
  discipline.

## The four subsystems, one line each

| # | Subsystem | Unlocks | Depends on |
|---|---|---|---|
| 1 | Shared pgid/session table | `GroupSpec::JoinGroup` on Windows (divergence 008) | Nothing else in this family |
| 2 | Arbitrary signal delivery | `Term`/`Int`/`Hup`/`Quit` beyond today's `Kill`-only (divergence 008) | Nothing else — found to need none of subsystem 1 |
| 3 | Real Stop/Cont | Cooperative `SIGTSTP`/`SIGCONT`, optionally `wait_job` observability | Extends subsystem 2's listener; observability needs a *separate* spawn-time pipe |
| 4 | Pty line discipline (Ctrl-Z/Ctrl-\ only, per its own scope reduction) | Job-control keys inside a ConPTY-hosted session | Needs a group-broadcast primitive composed from 1 + 2; has its own unresolved raw-mode-visibility hazard |

## The recurring blocker

None of the four has a named consumer. RFC v2 §3's standing rule is
that one must exist before work starts — the rule this whole family
has been honest about not clearing. That rule has been overridden
before, explicitly: PTY hosting (D13) and console acquisition (D9's
`rusty_naner` facet) both landed on the owner's own call to build
speculatively, accepting that risk in the open rather than waiting.
`Sandbox`'s privsep half and `CredentialVault` went the other way —
stayed donor-material-only once a session confirmed there was no live
gap or expressed migration desire. Both outcomes are precedented in
this codebase; nothing about msys-parity is different in kind.

## The decision

1. **Park indefinitely.** All five documents stay as a durable design
   record — the same fate `CredentialVault` and `Sandbox`'s privsep
   half already have. Revisit only if a real consumer shows up.
2. **Build one subsystem speculatively**, same posture as PTY/Console —
   explicit acceptance of the speculative-build risk, no consumer
   required. If this is the call, **which subsystem first** matters:
   subsystem 2 is the cheapest and most self-contained (no dependency
   on the others, and both 3 and 4 build on it rather than duplicating
   work), subsystem 1 is the one with an already-named, already-tracked
   API gap to close (`GroupSpec::JoinGroup`), subsystem 3 only makes
   sense once 2 exists, and subsystem 4 both depends on the most prior
   work and still carries an unresolved correctness question (its own
   document's open question 3) that arguably needs answering before
   *any* code, independent of sequencing.
3. **Wait for a named consumer** before any of this moves, the default
   this family has operated under so far — explicitly re-confirming
   that default rather than letting it stand unstated.

No recommendation is forced here beyond one observation: subsystem 2
is the smallest, cheapest way to learn whether this whole participation-
boundary model (rustils-aware processes only, no reach into arbitrary
Windows processes) is worth the investment at all, before committing to
1, 3, or 4 on top of it — worth weighing if option 2 is the answer.

## What this PR is asking for

One decision, recorded here once made — the same "Outcome" pattern
`docs/design-discussion-sandbox.md` and `docs/design-discussion-pty.md`
already use. Merge (or comment) this PR with a choice among the three
options above, or amend it directly; either way, this document should
end with an **Outcome** section once the call is made, matching every
other design-discussion doc in this family.
