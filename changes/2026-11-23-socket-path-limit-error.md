---
category: Fixed
changelog: A socket path over 107 bytes now says so, and rusty-mux-ctl takes a payload from stdin with --data-stdin, past the Windows command-line limit.
---
## 2026-11-23 - Windows limits: socket path error and --data-stdin (#1467)

- **Fixed:** a socket path longer than the 107 bytes a Unix socket takes (the same on Windows' AF_UNIX) now fails up front with the length and the limit, in the server, the clients and ctl.
- **Added:** `rusty-mux-ctl input --data-stdin` (the same `--NAME-stdin` form for any bytes parameter) reads the payload from stdin, so it is not bound by the Windows command-line limit of about 32 KB. One stdin flag per line; given with `--data` too, it is refused.
- **Known limitations:** the Windows run of these checks is left to CI.
