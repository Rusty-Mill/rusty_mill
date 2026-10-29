---
category: Fixed
changelog: rusty_tokio's LocalRuntime runs a woken spawn_local task at once; an otherwise idle loop used to wait up to its 50 ms park timeout, which delayed rusty_mux requests.
---
## 2026-10-29 - rusty_tokio: LocalRuntime wakes local tasks at once

- **Fixed:** `LocalRuntime::block_on` parked on the runtime's condvar, while a `spawn_local` task's wake only notified the `LocalSet`'s own, so an otherwise idle loop ran a woken local task only at its 50 ms park timeout. A request to an idle `rusty-mux-server`, keystrokes included, could wait up to 50 ms; in loopback release runs a request round trip on its Unix socket went from 50 ms to about 15 µs. Each schedule on the set now also wakes the runtime's loop, through a flag checked under the park lock so a wake that lands just before the park is not lost.
- **Fixed:** the `a_status_query_is_answered_in_order` mux test typed its second command while the terminal could still be raw, which the delay had hidden; each round now waits for its own end marker.
- **Known limitations:** a wake of the runtime's ordinary queue (`spawn`) is still caught only by the bounded park, as before.
