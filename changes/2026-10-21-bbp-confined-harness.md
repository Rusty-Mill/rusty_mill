---
category: Security
changelog: BBP harnesses can run confined under rusty_sandbox, unable to reach the task directory, with their MCP calls relayed to a broker outside the sandbox.
---
## 2026-10-21 - Confined BBP harnesses (Stage 3a)

- **Security:** a `bbp mod` launcher with `sandbox: {read, write, env}` runs its harness under `rusty_sandbox`: Landlock limits it to the system directories, the `bbp` binary, the declared paths and a fresh turn workspace under `--work`; seccomp allows internet sockets only; the environment is exactly the binding plus the declared variables. It cannot read `master` or the log, or run `bbp human`. A sandbox that could reach the task directory is refused before launch; one the kernel cannot enforce never runs the program.
- **Added:** `bbp mcp` relays over two named pipes when `BBP_MCP_RELAY` is set, to `bbp mcp-broker`, which the moderator starts outside the sandbox in the harness's process group.
- **Added:** Orch pool members accept the same `sandbox`, passed to BBP. `rusty_libc::fs::mkfifo`/`mkfifoat`.
- **Tests:** acceptance modes `bbp-confined` and `harness-confined` (through `rusty_orch run`) show the confined harness denied `master`, the log and writes to the directory while it plans to the gate through its relay; the unconfined control reads `master`.
- **Known gap:** the shipped provider launchers still read the task directory to validate their binding, so they run unconfined for now.
