---
category: Added
changelog: rusty_mux: a default socket, so the server and clients need no socket argument, and the server never removes a socket name it didn't make (C69 slice a).
---
## 2026-11-06 - rusty_mux default socket and safe socket names

- **Added:** with no socket named and no `$RUSTY_MUX_SOCKET`, `rusty-mux-server`, `rusty-mux`, `rusty-mux-gui`, `rusty-mux-ctl` and `rusty-mux-mcp` use `$XDG_RUNTIME_DIR/rusty_mux/default.sock` (`%LOCALAPPDATA%\rusty_mux\default.sock` on Windows). The server makes that directory 0700 and refuses one others can open.
- **Changed:** on Linux the server binds a temporary name and links it into place: it refuses to start where a server already answers or where something that is not a socket sits, replaces a socket nothing answers on, and on a clean stop removes the name only if it is still its own. The socket is 0600 as it is made. Login mode's front door uses the same rules.
- **Known limitations:** no default without `$XDG_RUNTIME_DIR`; auto-start and idle exit come in the next slice.
