---
category: Fixed
changelog: rusty-mux, rusty-mux-ctl and rusty-mux-server name the 107-byte limit when a socket path is too long, instead of the OS's "invalid argument".
---
## 2026-11-23 - A socket path over 107 bytes says so (#1467)

- **Fixed:** a socket path longer than the 107 bytes a Unix socket takes (the same on Windows' AF_UNIX) now fails up front with the length and the limit, in the server, the clients and ctl.
- **Known limitations:** the `rusty-mux-ctl --data` command-line limit (about 32 KB on Windows) is not changed here; it waits on an owner decision on how large payloads are passed.
