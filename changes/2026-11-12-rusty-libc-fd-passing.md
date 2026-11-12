---
category: Added
changelog: rusty_libc can pass descriptors over Unix sockets (SCM_RIGHTS), for rusty_mux's agent launch.
---
## 2026-11-12 - rusty_libc descriptor passing

- **Added:** `rusty_libc::socket::send_with_fds` and `recv_with_fds` send and receive up to 8 descriptors with a message on a Unix socket (`SCM_RIGHTS`). Received descriptors are close-on-exec from the start, and a control message that was cut short is reported. This is C75 part 1 (card #749), the API Nano chose in decision B. The mux side follows in its own PR.
- Arc 4 starts: pack D212 records Nano's go for C75, C76, C78 and the C79 spike.
