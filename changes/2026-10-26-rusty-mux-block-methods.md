---
category: Changed
changelog: rusty_mux block methods: `terminal.format` (replacing `capture`), `terminal.pwd` and `terminal.title` run through one generic block call, and `rusty-mux-ctl api list/describe` lists the methods with JSON Schema parameters.
---
## 2026-10-26 - rusty_mux block methods and API discovery

- **Added:** `Message::BlockCall { block, method, args }` (type 0x0f, fields 22 and 23), negotiated with `Features::BLOCK_CALL` (bit 7). The server dispatches on the block's type and checks the arguments against the catalog. An unknown method fails with the new `Code::MethodNotFound`. At most 16 arguments (`args::MAX_ARGS`) decode, and the server moves arguments under their catalog parameter names without copying them before validation.
- **Added:** terminal block methods in the catalog, named `TYPE.verb`: `terminal.format` (the screen after N history lines), `terminal.pwd` (the directory last reported with OSC 7, else the program's current directory) and `terminal.title`. ctl and the coming MCP server get them from the catalog.
- **Added:** `rusty-mux-ctl api list` prints every method; `api describe METHOD` prints it as JSON with its parameters as a JSON Schema. Neither needs a server.
- **Removed:** the `capture` request (type 0x0d, field 19), added in #1072 and replaced by `terminal.format` with the same output. `Features::INSPECT` now covers `list` only.
- **Known limitations:** ctl resolves a chain's targets against one `list` taken before the chain runs, so a target can't name an entity created earlier in the same chain; use the id that call prints. Text shaped like a UUID is always taken as a full id, never as a name. `terminal.pwd` falls back to the block's first program, not a job it started; there is no `api call`, since every method is already a ctl command.
