---
category: Added
changelog: rusty_mux actions and key bindings: a JSON config defines named actions, each a chain of catalog requests with `${arg}` placeholders, and key bindings; `rusty-mux-ctl do ACTION` runs one and `config check` validates the file.
---
## 2026-10-26 - rusty_mux actions and key bindings (C19)

- **Added:** `rusty_mux_script`, which parses a JSON config of `actions` and `binds` and checks it against the request catalog. An action is `{"name", "title"?, "args"?, "run": [{"call", "args"?}]}`, and a string value may hold `${arg}` for a declared argument, substituted in one pass (other `${...}`, such as `${HOME}`, stays as it is). A binding maps a chord sequence such as `ctrl+b>c` to an action or a catalog method. Values are checked as `rusty-mux-ctl` would check them: every parameter but an id (which can come from the environment) is required, and a literal must suit its type, so `cols: 0` or a missing `name` is a problem; a placeholder's value is checked once a binding gives it. Every problem is reported at once with where it is, such as `actions[0].run[1].args.cols`; unknown keys are problems.
- **Added:** `rusty-mux-ctl do ACTION [--ARG VALUE]...` runs an action's requests as one chain on one connection, as `a + b` does. `config check [PATH]` validates a file and `config list` prints its actions and bindings (`--json` for one object per line). Neither needs a server. The file is `$RUSTY_MUX_CONFIG`, else `rusty_mux/config.json` under `$XDG_CONFIG_HOME` or `~/.config`, read by every command, so there is nothing to reload.
- **Changed:** a chain resolves each request's targets against a fresh listing, so a request can name a session or window an earlier one in the same chain created.
- **Known limitations:** no language, conditions or event hooks; logic lives in programs driving `rusty-mux-ctl` or `rusty-mux-mcp`. No client reads the bindings yet. Actions are not MCP tools yet.
