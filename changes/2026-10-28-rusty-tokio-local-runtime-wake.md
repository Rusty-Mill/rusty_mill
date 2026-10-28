---
category: Fixed
changelog: rusty_tokio's LocalRuntime runs a woken spawn_local task at once instead of after its 50 ms park timeout, so each rusty_mux request no longer takes 50 ms.
---
## 2026-10-28 - rusty_tokio: LocalRuntime wakes local tasks at once

- **Fixed:** `LocalRuntime::block_on` parked on the runtime's condvar, while a `spawn_local` task's wake only notified the `LocalSet`'s own, so the loop slept out its 50 ms park timeout on every wake. Every request to `rusty-mux-server`, keystrokes included, waited 50 ms; a request round trip on its Unix socket now takes about 15 µs (loopback, release). Each schedule on the set now also wakes the runtime's loop, through a flag checked under the park lock so a wake that lands just before the park is not lost.
- **Fixed:** the `a_status_query_is_answered_in_order` mux test typed its second command while the terminal could still be raw, which the delay had hidden; each round now waits for its own end marker.
- **Known limitations:** a wake of the runtime's ordinary queue (`spawn`) is still caught only by the bounded park, as before.
