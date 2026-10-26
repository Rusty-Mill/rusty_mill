---
category: Changed
changelog: rusty-mux-ctl exit codes now follow Rex (3 target missing or ambiguous, 4 server unreachable), and id flags take a name, position, id suffix or path, defaulting to the block ctl runs in.
---
## 2026-10-25 - rusty-mux-ctl targets and exit codes

- **Changed:** `rusty-mux-ctl` exit codes: 3 is now a target that names nothing or more than one thing, and 4 is no server or a broken connection (it was 3). A protocol error, such as garbage from the server or a missing feature, now exits 1 (it was 4).
- **Added:** `Message::Failed` carries an optional `code` (field 21). `Code::NotFound` marks a request naming a session, window or block that doesn't exist; ctl exits 3 on it. An unknown code decodes as none.
- **Added:** id flags take a target besides a full id: a case-insensitive name, a position counting from 1, or the last four or more characters of an id, tried in that order. A path `SESSION/WINDOW/BLOCK` names the parents; a shorter path drops leading parts. ctl resolves targets against one `list` before sending the chain.
- **Added:** the server starts each block's program with `RUSTY_MUX_SESSION`, `RUSTY_MUX_WINDOW`, `RUSTY_MUX_BLOCK` and `RUSTY_MUX_SOCKET`. ctl fills a left-out id flag from them, and windows and blocks named without a session are looked up in `RUSTY_MUX_SESSION`.
- **Changed:** the server lists sessions in creation order, not id order, so positions are stable. `Mux::add_block` adds a block under an id the caller chose.
- **Known limitations:** blocks have no names yet, so a block target is a position, an id suffix or a full id.
