# 003 — Track W: a provenance swap, not a depth swap

Encountered wiring `rusty_win32` in as the Track W backend for
`sys::fileio::read`/`write` (platform-windows, D-15) — the Windows
counterpart of note 002's Track P work, deliberately mirrored onto the
same first call family so the two adoptions could be compared directly.

## The asymmetry the mirror hides

Track P buys a **lower tier**: `rusty_libc` issues the `read` syscall
itself, so glibc leaves the picture entirely and the kernel ABI becomes
the floor. The obvious reading of "do the same thing on Windows" says
Track W should get under `kernel32` the same way — and it can't, because
there is nothing down there to stand on. Windows publishes no stable
syscall numbers; the `ntdll` stubs are renumbered between builds on
purpose. A documented DLL export **is** the bottom of the supported
stack. So `windows-sys`' `ReadFile` and `rusty_win32`'s `ReadFile` are
not two tiers — they are two *declarations of the same import*, and at
runtime both land on the identical `kernel32!ReadFile`.

That makes Track W a different kind of trade than Track P, and it is
worth naming rather than letting the symmetric feature name imply
otherwise:

| | Track P (D-12) | Track W (D-15) |
|---|---|---|
| What changes | the tier — libc wrapper → raw syscall | the *provenance* of the binding |
| Floor after the swap | kernel ABI | the same documented DLL export |
| What it buys | removes a userspace dependency from the path | a hand-written, reviewed declaration; no `windows-targets` import-lib machinery; `no_std`-capable |
| What it cannot buy | — | anything below the export |

This is why D-1 (windows-sys is the floor) survives D-15 intact instead
of being superseded the way D-2 was softened by D-12. Track W is opt-in
because it is a swap between peers, not a descent.

## The blocker was Cargo, not code

The interesting failure had nothing to do with Windows. `rusty_win32`
was `edition = "2024"`, and adding it as an **optional** dependency with
the feature **off** still broke the 1.75 MSRV CI leg outright:

```
error: failed to get `rusty_win32` as a dependency of package `platform-windows`
Caused by: feature `edition2024` is required
```

Cargo resolves optional dependencies and parses their manifests
regardless of feature selection — the feature gate decides what gets
*compiled*, not what gets *read*. So a dependency's `edition` is a
compatibility constraint on every consumer that merely lists it, in a
way its `rust-version` is not. Track P never surfaced this because
`rusty_libc` happened to already be edition 2021 with `rust-version =
"1.88"`; the split between those two fields was doing load-bearing work
nobody had had to notice yet.

Fixed upstream rather than worked around here (rusty_win32 is a sibling
repo, not a third-party crate): edition 2021, `rust-version = "1.88"`,
and the two things the edition drop would have quietly taken with it
re-asserted explicitly —

- `[lints.rust] unsafe_op_in_unsafe_fn = "deny"`, which edition 2024
  makes the default and which is how that crate is already written; and
- `rustfmt.toml` pinning `style_edition = "2024"`, because rustfmt's
  style edition otherwise **follows the crate edition**. Without it the
  edition field alone reflows the entire codebase — `use`-list sort
  order flips, single-line `if … { A } else { B }` expands, trailing
  comments after a wrapped expression re-indent to the expression's
  column. A packaging change should not be a formatting change.

The 1.88 floor was then measured rather than inherited: `impl Default
for *mut T` stabilized in 1.88, and `#[derive(Default)]` on the
OVERLAPPED/handle-bearing structs needs it — 1.87 fails, 1.88 is clean.
It now has its own CI job upstream, since a `rust-version` nobody
compiles against drifts silently, and this one is load-bearing for us.

## The error path: note 002's lesson, reached from the other side

Track P's rule was "the code must flow from the returned value, because
a raw syscall never touches the thread-local `errno` at all." Track W
reaches the same rule from the opposite mechanism: `GetLastError`'s slot
**is** still written — `rusty_win32` calls it, exactly like the
windows-sys arm does. What changes is *who is holding the receipt*. The
wrapper read the slot at the only instant it was valid and returned a
`Win32Error` by value; re-reading `GetLastError` at our call site would
be a live race, because any intervening Win32 call — including ones the
wrapper made on its own way back out — overwrites it.

So `trackw_err(op, Win32Error)` sits beside `last_win32_err(op, path)`
for precisely the reason `trackp_err` sits beside `os_err`, and both
arms then converge on the same `kind_of_win32` table and
`OsCode::Win32`. `PlatformError` is bit-identical either way, which is
what lets the entire platform-windows suite re-run under `--features
track-w` as the equivalence test instead of needing a parallel one.

Generalized: *the thread-local last-error slot is never the authority
once a wrapper stands between you and the call.* Track P's version of
that sentence is only the special case where the slot is also empty.

## Two things that did *not* mirror

**The `unsafe` block does not disappear.** In Track P it did:
`rusty_libc::fd::read` takes `&mut [u8]` and derives the pointer/length
pair itself, so the caller had nothing left to assert.
`rusty_win32::fs::read_file` also takes `&mut [u8]` — but it is still an
`unsafe fn`, because its first argument is a bare `RawHandle` with no
ownership or liveness type attached. The obligation moved rather than
vanished, and what discharges it is our own `OwnedWinHandle` invariant
(constructed only from a freshly-returned valid handle, closed exactly
once on drop, `&self` proving it hasn't). That is the crate's own
hand-rolled value from D-1 doing its job — and a reminder that a safe
signature on a `&[u8]` says nothing about the *handle* parameter beside
it.

**The length clamp had to move to our side.** `read_file`/`write_file`
pass `buf.len() as u32` straight through, so an oversized slice *wraps*
— a 4 GiB + 1 buffer would read one byte, or zero. The windows-sys arm
saturates inline (`u32::try_from(…).unwrap_or(u32::MAX)`). Clamping the
slice before the call keeps the two arms' contract identical; short
reads were already the contract, so this bounds one call rather than
changing behavior. Worth stating plainly because it is the general
hazard of adopting a thinner wrapper: the parts it declines to do are
not documented as gaps, they are simply absent, and they become yours.
