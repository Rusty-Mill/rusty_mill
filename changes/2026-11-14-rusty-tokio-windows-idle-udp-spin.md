---
category: Fixed
changelog: rusty_tokio on Windows no longer spins a core on an idle socket (UDP sockets are always writable and were re-polled in a loop).
---
## 2026-11-14 - rusty_tokio Windows reactor idle spin

- **Fixed:** the Windows reactor re-armed its AFD poll right after every completion. A socket that stayed ready (an idle UDP socket is always writable) completed again at once, so one reactor core spun, in release too. A direction that fires while still marked ready is now left out of the next poll, and watched again when the caller clears it after `WouldBlock`, like the edge-triggered epoll and kqueue backends. A poll already running is cancelled and resubmitted with the new mask.
- Test: `an_idle_udp_socket_does_not_spin_the_reactor` (Windows only). Found by mesh spike A on Beast (pack Q61).
