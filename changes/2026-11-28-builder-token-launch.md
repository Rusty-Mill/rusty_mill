---
category: Added
changelog: `rusty_orch builder run` can mint the session's gateway token and hand it to the jailed agent, with `--token-key`, `--token-kid` and `--lens-policy`.
---
## 2026-11-28 - orch: builder sessions carry a gateway token

- **Added:** `rusty_orch builder run --token-key <pem> --token-kid <kid> --lens-policy <lens>.toml` (Let Em Loose #1382 slice 2, D232, D233). The three flags go together. With them, orch signs a token for the session using `orch-token`, writes it to `<state>/tokens/<claim>/<n>/token` (0600 in a 0700 directory), and fills the `jail.json` `token` slot with that file and `LEL_TOKEN`. `lel-jail` then hands the token to the agent.
  - `sub` is `<board>/<claim>/<lens>/<n>`. `lel.task` is `--task`. The lens is the policy file's `lens`, and `lel.policy` is the file's sha256. `iss` is `lel-orch@<uname nodename>`. The `jti` is 16 random bytes from `rusty_rand`. The budget is the config's builder wall clock.
  - Launch is refused, before admission, for an unreadable or non-P-256 key, or a policy whose `lens` isn't its file name or whose `role` isn't `builder`. It is also refused when a `--read` root reaches the token file; `JailSpec::validate` checks that.
  - The session directory `<n>` is created exclusively (0700) and the file with `create_new` (0600). If that directory already exists, it is left by an earlier run of the same session number: launch is refused with its path, and nothing there is removed or reused. The guard that removes the file and directory exists only once this process has created them, so a failed create or write cleans up only its own files, and every exit path does. Only `jti` and `sub` are logged, to stderr.
- **Changed:** A partial set of the three flags is a usage error. With none, the session has no token, as before (D187).
- **Known limitations:** Nothing in the jail uses the token yet. The agent's MCP config doesn't point at the gateway (slice 4, #1412), and early revocation is slice 3, which waits on Nano.
