# Decision needed: `unix_listen`'s stale-reclaim gate on Windows misses one dead-listener case

Not a design document — a decision request, the same shape
`docs/decision-request-detach-liveness.md` uses. Opened against a real
failure hit by the not-yet-pushed `rusty_prime_agent` daemon-restart
harness (2026-08-11): its `tests/supervisor_restart_recovery.rs` force-kills
the running supervisor process, then rebinds its `daemon.sock` — and on
Windows, that rebind sometimes fails with a bogus `os error 0
(Uncategorized)` instead of either succeeding or a real `AddrInUse`.

## Repro condition, as traced from the harness

Confirmed to reproduce **only** when the process being killed had also
made an outbound `AF_UNIX` connection to a *different* path (the harness's
own worker-private socket) before dying — a throwaway-path bind in the
same process state succeeds cleanly, so this is specific to reclaiming a
path the dead process both listened on **and** had other live socket
state alongside, not a general "bind after a crash" failure.

## Root cause, traced through the stack

`rusty_prime_agent` → `rusty_tokio::io::UnixListener::bind` →
`platform_windows::WindowsUnixListener::bind` →
`sys::net::unix_listen`, this crate's own code. `rusty_tokio` adds no
retry/reclaim logic of its own on top — `io/unix.rs`'s own doc comment on
`UnixListener::bind` says as much ("rustils' own `unix_listen`
distinguishes 'stale' from 'still live'"), so this is entirely a
`rustils` question.

`unix_listen`'s stale-cleanup retry (module doc, `sys::net.rs:14-23`) is
gated on the failed `bind` reporting `ErrorKind::AddrInUse` — the
documented, and on an idle system reliable, signal that Winsock can't
otherwise distinguish a path a live listener holds from one a dead
listener left behind. `wsa_kind` maps every *other* Winsock error code to
`ErrorKind::Other`, including the impossible-on-paper code `0`
(`WSAGetLastError() == 0`, i.e. "no error") that `wsa_err` would return if
`bind`'s own `SOCKET_ERROR` return value is ever paired with a last-error
slot that, for whatever reason, doesn't hold a real Winsock code at that
moment.

**Hypothesis, not yet independently confirmed against real Windows in
this session** (see Verification below): in the exact race window right
after `TerminateProcess` on a process that held *two* live `AF_UNIX`
handles (the listener plus the outbound connection), Windows appears to
still be mid-teardown of that process's socket table when a fresh `bind`
on the listener's old path lands — the call fails (`SOCKET_ERROR`) but
`WSAGetLastError()` reads back `0` instead of the expected
`WSAEADDRINUSE`. Because `unix_listen`'s gate checks only `AddrInUse`,
this anomalous failure skips the stale-socket probe (`is_stale_socket`)
entirely and bubbles up raw as `os error 0` — the bug the harness hit.

This reads as a real Windows/afunix.sys teardown-timing quirk (a
dead-listener path failing to reliably report `WSAEADDRINUSE` in this
specific race), not a `rusty_prime_agent`- or `rustils`-side logic bug in
isolation — but per this repo's own rule (`docs/divergences.md`'s header:
"a divergence may cite only an OS limitation, never implementation
convenience"), that claim needs the same kind of real-`windows-latest`
evidence divergence entries #011/#012/#015 all cite before it earns a
numbered entry, which is why this is a decision request instead of one.

## Decision

**Widen `unix_listen`'s stale-reclaim gate**, rather than special-casing
this in `rusty_tokio` or in `rusty_prime_agent`'s own transport layer.
Reasoning for fixing here specifically:

- The bug is entirely inside this crate's own dead-listener detection —
  `rusty_tokio` has no reclaim logic of its own to diverge from, and a
  harness-side workaround (a probe-based liveness check papering over
  the library's own misclassification) would leave every other consumer
  of `rustils`' Windows `AF_UNIX` support to hit the identical bug.
- `is_stale_socket`'s probe-connect is still the sole authority for
  actually deleting the leftover path — widening *which failures reach
  that probe* cannot itself misclassify a genuinely live listener as
  stale, since the probe's own `WSAECONNREFUSED` check is unchanged.

**Implementation** (`crates/platform-windows/src/sys/net.rs`): a new
`is_stale_bind_candidate(e: &PlatformError) -> bool` gate,
`e.kind == ErrorKind::AddrInUse || matches!(e.os, OsCode::Win32(0))`,
replacing the bare `e.kind == ErrorKind::AddrInUse` check `unix_listen`'s
match arm used before. A freshly created, not-yet-bound socket's `bind`
cannot legitimately fail with a success code — treating that specific
nonsensical pairing as an `AddrInUse`-equivalent candidate is a narrow,
safe widening, not a general loosening of when reclaim is attempted.

### Options considered

1. **Leave `unix_listen` untouched; work around it in
   `rusty_prime_agent`'s `transport::Listener::bind_with_retry`** (using
   the harness's own `probe()` ping/pong to authoritatively check
   liveness on any bind failure, then force-remove the file). **Not
   chosen**: the harness's own task brief for this is explicit that this
   is the fallback only if the root cause is genuinely unfixable
   upstream, and it plainly isn't — the misclassification is inside this
   crate's own gate, fixable at the source instead of papered over in
   every consumer.
2. **Drop the `ErrorKind` gate entirely; probe on any `bind` failure.**
   Rejected as broader than the evidence supports: every other bind
   failure this crate can produce (invalid path, permission denied, path
   too long) has a real, actionable `ErrorKind` already, and folding
   those into the stale-probe path too would mean a permission error, for
   instance, silently attempts a throwaway connect before surfacing —
   harmless given `is_stale_socket`'s own connect-refused gate, but wider
   than the one anomaly this decision has actual evidence for. **Not
   chosen**, revisit only if more anomalous codes turn up.
3. **Widen the gate to include `OsCode::Win32(0)` specifically** (chosen
   above). Matches the actual evidence (one specific impossible-on-paper
   code, traced to one specific race), stays otherwise as conservative as
   the existing `AddrInUse`-only gate.

## Verification

**Not yet run against real Windows in this session** — this branch was
authored in a Linux-only sandbox with no native Windows execution
available; the change is `cargo check`/`cargo clippy --target
x86_64-pc-windows-gnu`-clean and `cargo fmt`-clean, matching this crate's
own "developed from a Linux host, verified for real on CI's
`windows-latest` leg" discipline already established for every other
Windows-only behavior in this crate (divergences #011, #012, #015; this
crate's own `lib.rs` module doc). A new live regression test,
`crates/platform-windows/tests/stale_reclaim_process.rs`
(`rebind_after_forced_kill_of_a_listener_that_also_held_an_outbound_connection`),
reproduces the harness's exact repro shape — bind a listener, connect out
to a second live listener from the same process, force-kill that process
from a genuinely separate parent process (`TerminateProcess` via a
re-exec'd helper, not a same-process socket close, since the mechanism
under test is specifically process-teardown timing), then assert a fresh
bind at the original path succeeds within a bounded window. This test is
CI-only until a `windows-latest` run actually executes it.

## Open questions for the owner

- **This fix is unverified beyond visual review and cross-compile
  checks.** Whoever merges this should confirm the new
  `stale_reclaim_process.rs` test (and the pre-existing
  `windows_unix_conforms` parity test) both pass on a real
  `windows-latest` CI run before treating this as closed, and before
  promoting the hypothesis above into a numbered `docs/divergences.md`
  entry — this document intentionally stops short of that until there's
  real-hardware evidence to cite, per the registry's own rule.
- If CI shows the `os error 0` failure *still* reproduces even with this
  gate widened, the next-most-likely spot to look is `is_stale_socket`'s
  own probe: the same teardown-timing race could in principle also affect
  the probe `connect`'s own error code, in which case the fix needs to
  widen `is_stale_socket`'s `WSAECONNREFUSED` check the same way, not
  just `unix_listen`'s outer gate.
