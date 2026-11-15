---
category: Fixed
changelog: A ts-net stream no longer loses the tail of a reply when the connection reaches TimeWait and Closed before a slow reader has read it.
---
## 2026-11-15 - ts-net: keep unread data past TimeWait (follow-up to #1730)

- **Fixed:** when we close first, the peer replies at length and then finishes, and the reader is slow, smoltcp wipes the socket's receive buffer when TimeWait ends. The stack reaped the connection then, so the reader saw an early EOF after only what was already queued (19,880 of 70,000 bytes in the new test). Once the peer's FIN is in, the stack now moves what is left of the receive buffer (at most 64 KiB) into `Conn::pending_rx` (now a queue) and reaps only when that is empty.
- **Tested:** `a_reply_survives_time_wait_while_the_reader_is_slow` fails on main (19,880 of 70,000 bytes) and passes with the fix; the other ts-net tests and the hub transfer test pass. The new test sleeps 11 s to outlast smoltcp's 10 s TimeWait.
