---
category: Added
changelog: orch gains `orch-fs-shell`, the sandbox fs/shell MCP server for builders that bring no shell of their own.
---
## 2026-11-27 - orch: `orch-fs-shell`

- **Added:** `orch-fs-shell` (Let Em Loose #1379, 726-sandbox-design §6, D225). `orch-fs-shell --root DIR --mode builder|critic` serves MCP over stdio with `read`, `list`, `stat`, `write` (builder only) and `exec` over one session root.
  - Paths are relative to the root. Absolute paths, `..` (either separator), drive prefixes and NUL are refused before any I/O; every path is then resolved and refused if its real location is outside the root, so a symlink to `/proc` or out of the worktree is refused too. A write lands only on an existing target inside the root, or a new name in an existing directory inside it.
  - `exec` runs argv with no shell, in a `cwd` inside the root, with optional stdin, a 60 s default timeout capped at 600 s, and the child killed at its timeout. File contents, stdout and stderr are each capped at 1 MiB, with `truncated` set; non-UTF-8 output is returned lossily and flagged.
  - A pure core (`core::handle` over the `Workspace` port) and a std adapter (`FsWorkspace`); no new third-party dependency.
  - **Not a sandbox.** `--mode critic` withholds the `write` tool and nothing else. `exec` runs any program with the server's own rights, so a critic's commands can write files, reach paths outside the root, and leave descendants running. The path checks cover only the paths named in tool calls. Read-only critic execution, filesystem confinement and descendant cleanup must come from the jail that launches the process. `--help`, every usage error, the crate docs and the `exec` tool description say so.
- **Known limitations:** The gateway does not launch it yet, and this PR adds no gateway registration or launch. That later change calls the feature confined only with evidence that the external jail enforces read-only critic execution and owns descendant cleanup. The path check and the I/O are not atomic, so the path checks are a second fence, not the boundary. A grandchild that keeps the output pipes open after a timeout is left running. The answer waits at most 2 s more for its output, but the reader threads and pipes stay alive until it closes them, so this server does no descendant cleanup.
