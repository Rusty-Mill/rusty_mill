# Design discussion — pty line discipline (msys-parity subsystem 4, deeper scope)

Not a decision record. `docs/design-discussion-msys-parity.md`'s own
open question 5 already named the smaller of two options for subsystem
4 — a shim adding ICANON/ISIG-style signal-generating-character
emulation to the already-landed ConPTY backend (D13), rather than a
second, parallel named-pipe pty backend replacing it — without scoping
that shim further. This document does, re-reading Cygwin's actual line
discipline (not the parent doc's paragraph-length summary). With this
document, all four subsystems named in the parent document have
implementation-depth scoping.

## What Cygwin's real line discipline does

`fhandler_termios::line_edit`
(`winsup/cygwin/fhandler/termios.cc:514`) — shared code between
`fhandler_console` *and* `fhandler_pty_master`, processing every byte
written toward a pty/console slave — does three genuinely separate
things per byte, in order:

1. **`process_sigs`** (`termios.cc:322`): checks the byte against
   `ti.c_cc[VINTR]`/`VQUIT`/`VSUSP` (gated on the `ISIG` termios flag)
   and maps to `SIGINT`/`SIGQUIT`/`SIGTSTP`. Delivery is
   `ttyp->kill_pgrp(sig, pgid)` — **a whole-process-group broadcast**,
   reading `pgid` from the tty's own shared struct (`termios.cc:325`,
   `pgid = ttyp->pgid`) — this is a live consumer of exactly the shared
   pgid table `docs/design-discussion-msys-pgid-table.md` scoped
   (subsystem 1), not a hypothetical dependency.
2. **`process_stop_start`** (`termios.cc:485`): `VSTOP`/`VSTART`
   (Ctrl-S/Ctrl-Q) flow control — purely local (`ttyp->output_stopped`,
   consulted by the same process's own write loop), no cross-process
   delivery involved. Out of this document's scope; noted for
   completeness, not pursued further.
3. **Canonical-mode editing** (`VERASE`/`VKILL`, local echo) and
   `ICRNL`/`INLCR`/`IGNCR` CR/LF translation, entirely in Cygwin's own
   userspace state machine (`termios.cc:552` onward) — because Cygwin's
   pty (named pipes, `docs/design-discussion-msys-parity.md`'s own D13
   citation) is never attached to a real Windows console at all. There
   is nothing underneath it to do this work for free.

**Ctrl-C is special-cased beyond `process_sigs`'s own signal mapping**
(`termios.cc:341-385`): on the literal byte `\003`, Cygwin *also* walks
every process on the system and calls
`GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)` for ones attached to the
same console — a second, independent delivery path specifically
because a **non-Cygwin-aware native Windows process** in the same
foreground group can't receive Cygwin's own named-pipe `sig_send`
(D9/D13's own "participation boundary": only cooperating processes get
anything from the Cygwin-specific channel) but *can* receive a real
Win32 console-control event. This is a concrete precedent for bridging
rustils-aware and native processes in a mixed group, addressed further
below — and a real limit on that bridge, since `CTRL_C_EVENT` is the
*only* one of the three deliverable console-control identities
(divergence 003) with no counterpart for `VQUIT`/`VSUSP` at all: there
is no `CTRL_QUIT_EVENT`/`CTRL_STOP_EVENT`. A native process in a mixed
group can be interrupted; it can never be sent a Cygwin-style
quit-with-core-dump or job-control-suspend, by any means, cooperating
or not — a harder limit than the one Cygwin's own Ctrl-C bridge works
around.

## The finding that shrinks this subsystem's real scope

Cygwin's pty has no Windows console underneath it — it must
reimplement canonical-mode editing, CR/LF translation, and Ctrl-C
handling entirely itself because there is nothing else there. **This
crate's `platform::pty` (D13) is structurally different: it is
ConPTY-based, and ConPTY hosts a real (virtual) Windows console under
the hood.** A pty-hosted child that hasn't put its own inherited
console handles into raw mode should already get, from Windows itself,
with no shim involved: canonical-mode line editing (`ENABLE_LINE_INPUT`
— backspace/erase), and Ctrl-C delivered as a real `CTRL_C_EVENT`
(`ENABLE_PROCESSED_INPUT`, Windows' own default). If that holds, the
Cygwin-scale reimplementation this subsystem's one-paragraph parent
description implied isn't the real shape of the work at all — the
*only* signal-generating characters with **no Windows console
equivalent whatsoever** are `VSUSP` (Ctrl-Z → job-control suspend) and
`VQUIT` (Ctrl-\ → quit-with-core-dump). Those two, not the whole line
discipline, are this subsystem's real, narrow target.

**This needs live verification, not just inspection** — the same bar
`docs/design-discussion-pty.md` held itself to before trusting ConPTY's
own EOF/teardown behavior, and nothing in the already-landed PTY work
(`crates/platform-windows/src/sys/pty.rs`,
`docs/design-discussion-pty.md`) touches console-mode/raw-vs-cooked
behavior for a ConPTY-hosted child at all — this document is the first
place in this codebase that question comes up. Stated as a real
uncertainty, not asserted as settled: whether a ConPTY-hosted child's
own `SetConsoleMode` calls on its inherited std handles genuinely gate
Ctrl-C/line-editing behavior the identical way a real, non-pseudo
console does, or whether ConPTY's own emulation layer does something
different that this document's assumption doesn't hold for.

## The concrete correctness hazard this finding surfaces

If the assumption above is right, it creates a real problem this
document does not resolve: **the master-side shim has no visibility
into whether the hosted child is currently raw or cooked.** `enter_raw`
(`sys::console::enter_raw`, already landed) clears
`ENABLE_PROCESSED_INPUT` specifically so a raw-mode reader sees Ctrl-C
as ordinary input instead of being interrupted by it — its own doc
comment says so explicitly. A pty-hosted child calling `enter_raw` on
itself (e.g. a full-screen editor wanting Ctrl-Z as a keybinding, not a
suspend request) is doing the process-private, dynamically-toggled
equivalent of clearing `ISIG` — exactly what real POSIX raw mode does,
and exactly why a raw-mode program can capture Ctrl-C/Ctrl-Z as
ordinary bytes on a real Unix tty. **A master-side shim that
unconditionally intercepts `VSUSP`/`VQUIT` bytes regardless of the
child's own raw/cooked state would break that** — forcibly suspending
a program that explicitly asked to see Ctrl-Z itself. Cygwin's own
`line_edit` doesn't have this problem because it *is* the process
setting `ti.c_lflag`'s `ISIG` bit in the first place (there's no
separate master/slave-console duality to desynchronize); a ConPTY-based
design does, because the master (this shim) and the slave (the hosted
child's own console-mode state) are genuinely different vantage
points with no existing plumbing connecting them. Whether that
plumbing can exist at all — some way for the shim to observe the
current state ConPTY is presenting to its hosted child, or an
acceptable design that doesn't need to — is this document's most
important open question, not a minor implementation detail.

## Proposed shape, contingent on the open question above

A `PtyMaster` decorator implementing `platform::pty::PtyMaster` over
the real `WindowsPtyMaster` (D13) — object-safe trait, so wrapping is
free — intercepting `write()` (the direction carrying keystrokes into
the pty) to scan for `VSUSP`/`VQUIT` bytes before forwarding the rest
unchanged. `PtyMaster::write`'s own contract is already synchronous/
blocking (`docs/design-discussion-pty.md`'s own finding), so this is an
ordinary in-process byte scan on every call — no new thread, unlike
subsystems 2/3's listener threads (which exist because they're on the
*receiving* end, in a different process).

**On a match, this subsystem needs a capability none of subsystems 1–3
built**: group-broadcast delivery (`kill_pgrp`'s shape), not
point-to-point. Subsystem 2's `PeerSignalSource` delivers to one known
pid; subsystem 3's suspend/resume targets one process. Neither
iterates "every pid currently in this pgid." The natural composition,
not a fourth independent mechanism: a `deliver_to_group(pgid, signal)`
that walks subsystem 1's pgid table for matching entries and calls
subsystem 2's existing per-pid delivery for each — three already-scoped
pieces combined, not new infrastructure.

**The native-process bridge**: since ConPTY-hosted children *are* real
(virtually-consoled) Windows processes, the same
`GenerateConsoleCtrlEvent`-based bridge Cygwin uses for Ctrl-C applies
here for free, for Ctrl-C specifically — but as noted above, it has no
counterpart for `VQUIT`/`VSUSP` at all. A native, non-`platform`-aware
process sharing a pty-hosted job's process group is unreachable for
those two, permanently, by construction (no Win32 event exists to
carry them) — a harder limit than any divergence this codebase has
registered so far, since it isn't "we chose not to implement this," it
is "there is no OS mechanism to implement it with."

## Open questions for the owner, not decided here

1. **Still no named consumer** — same as every item in this family.
2. **Does ConPTY-hosted line editing/Ctrl-C actually work the way this
   document assumes?** The load-bearing question. If it doesn't — if
   ConPTY's emulation is opaque to or different from ordinary
   `SetConsoleMode` gating — this document's entire scope reduction is
   wrong and subsystem 4 is closer to Cygwin's original size after all.
3. **Is there any way for a master-side shim to observe a hosted
   child's current raw/cooked (`ISIG`-equivalent) state**, or does the
   correctness hazard above mean `VSUSP`/`VQUIT` interception has to be
   opt-in/configured by whoever calls `Pty::spawn` rather than always
   active — the pty-hosting caller stating up front "this session wants
   job-control keys interpreted," accepting that a raw-mode child
   inside it can't locally override that? A materially different, and
   possibly more honest, design than trying to auto-detect state that
   may not be observable at all.
4. **Does `deliver_to_group` belong on subsystem 1's table API, on
   subsystem 2's `PeerSignalSource`, or as its own thing** — a real
   design-ownership question now that this document has shown a
   concrete consumer for it, where subsystems 1–3's own documents had
   none.
5. **Is the native-process `VQUIT`/`VSUSP` gap (no bridge exists at
   all) acceptable, or does it change whether this subsystem is worth
   building** for a mixed rustils/native process group specifically —
   worth asking given it's strictly worse than what Cygwin itself can
   offer in the same situation.

## What this document does not decide

Whether to build this, whether the ConPTY-gating assumption holds,
or how the observability hazard in question 3 gets resolved (if it
can be). Same posture as every document in this family: input to an
owner's call, not the call itself. All four subsystems
`docs/design-discussion-msys-parity.md` named now have this level of
scoping; none of the five documents in this family have decided to
build anything.
