---
category: Added
changelog: orch gains `orch-fs-shell`, the sandbox fs/shell MCP server for builders that bring no shell of their own.
---
## 2026-11-27 - orch: `orch-fs-shell`

- **Added:** `orch-fs-shell` (Let Em Loose #1379, 726-sandbox-design §6, D225). `orch-fs-shell --root DIR --mode builder|critic` serves MCP over stdio with `read`, `list`, `stat`, `write` (builder only) and `exec` over one session root.
  - Paths are relative to the root. Absolute paths, `..` (either separator), drive prefixes and NUL are refused before any I/O; every path is then resolved and refused if its real location is outside the root, so a symlink to `/proc` or out of the worktree is refused too. A write lands only on an existing target inside the root, or a new name in an existing directory inside it.
  - `exec` runs argv with no shell, in a `cwd` inside the root, with optional stdin, a 60 s default timeout capped at 600 s, and the child killed at its timeout. File contents, stdout and stderr are each capped at 1 MiB, with `truncated` set; non-UTF-8 output is returned lossily and flagged.
  - A pure core (`core::handle` over the `Workspace` port) and a std adapter (`FsWorkspace`); no new third-party dependency.
- **Known limitations:** The gateway does not launch it yet: how a per-session stdio target gets its session's root and jail is a new gateway setting, waiting on Nano. The path checks are defence in depth; the check and the I/O are not atomic, so the boundary is `lel-jail` (Landlock), which wraps the process. A grandchild that keeps the output pipes open after a timeout is left running; its output stops being read 2 s after the kill.
