---
category: Added
changelog: rusty_mux config reload: the server reads new-block defaults from the config file's `server` section, again on `reload-config`, and announces `config-reloaded`.
---
## 2026-11-01 - rusty_mux config reload (C38)

- **Added:** the config file (`$RUSTY_MUX_CONFIG`, else `rusty_mux/config.json` under `$XDG_CONFIG_HOME` or `~/.config`) takes a `server` section: `command` and `args` replace the user's shell, and `cwd` (an existing absolute directory) the start directory, for each new block whose request does not name its own. The server checks only that section; the rest of the file stays ctl's. `rusty-mux-server --config FILE` names another file. A missing file sets nothing; a bad one stops the server before it starts.
- **Added:** the `reload-config` request (ctl `reload-config`, and the MCP tool of the same name) reads the file again. A bad, missing or non-regular file is refused, with every problem listed, and the server keeps its last settings. The file is read off the server's thread, one reload at a time (another asked meanwhile is refused), so a stalled filesystem holds up no other client. Blocks already running keep their program.
- **Added:** `Event::ConfigReloaded` (kind 30) follows each reload that took. The request and the event sit behind the new `Features::CONFIG_RELOAD` (bit 25); a client without it never receives the event, live or on replay. `rusty-mux-ctl events`, `watch`, `wait --event` and `on` rules know it as `config-reloaded`.
- **Changed:** `rusty_mux_server::Config` has a new `config_file` field, and the server now depends on `rusty_mux_script` (a workspace crate; no new third-party dependency).
- **Known limitations:** no signal reloads the config; a hangup still ends the server as before.
