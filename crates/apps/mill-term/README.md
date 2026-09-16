# mill-term

MSYS2 & Git Bash replacement environment launcher for Rusty Mill: hosts a
real [`rusty_term`](../rusty_term)-rendered terminal session running
[`rush`](../rush), with a real [`rusty_git`](../rusty_git)-powered
repo-status banner, and the sibling `rgit`/`rsed`/`rawk` tool binaries put
on the child's `PATH`.

## What's real

Previously `mill-term` declared `rusty_term`/`rusty_git`/`rusty_text` as
dependencies but called none of them — just `Command::new("rush").status()`
with no PTY. Now:

- **Terminal hosting**: calls `rusty_term::runtime::run` directly (the same
  function `rusty_term`'s own binary uses), constructing a real `Backend`,
  `Grid`, and `Config` with `shell` set to a resolved `rush` binary — a real
  PTY-backed terminal session, not a bare subprocess status wait.
- **Git status banner**: `rusty_git::Repository::open`/`current_branch`/
  `status` power a real "current branch + pending change count" line at
  startup.
- **Tool discovery**: `resolve_tool`/`find_sibling_binary` locate `rush`,
  `rgit`, `rsed`, `rawk` — first via `PATH` (the "properly installed" case),
  falling back to a sibling dev-workspace build (`../rush/target/{debug,
  release}/rush.exe`, etc., since these repos are siblings under one parent
  directory, not a single Cargo workspace with one shared `target/`). The
  found tools' directories are prepended to the hosted session's `PATH`.

`rusty_text` is intentionally **not** a Cargo dependency: there's no
non-contrived use for a sed/awk engine in a terminal launcher's own startup
logic. Its `rsed`/`rawk` binaries are still made reachable via the same
sibling-binary lookup as `rgit`, without a Rust-level dependency on the
crate that builds them.

## A real bug this integration surfaced and fixed

Wiring in the real `rusty_git` status banner revealed that `rusty_git`'s
`status`/`add` didn't respect `.gitignore` at all — they walked entire
`target/` build directories (hundreds of MB, thousands of files) on every
call. In this very repo that made `rgit status` take 30+ seconds (looking
like a hang). Fixed in `rusty_git` itself (`src/gitignore.rs`, a real if
minimal `.gitignore` matcher); `rgit status` here now completes in ~0.1s.

## Testing note

`cargo test` covers the pure logic (`find_sibling_binary`/`resolve_tool`/
`augmented_path`) against the real sibling binaries built in this
workspace. The actual interactive terminal session **cannot be
end-to-end tested from a non-interactive/piped shell**: running it (or the
unmodified `rusty_term` binary itself) under piped stdio with no real
attached console fails with `Os { code: 6, "The handle is invalid" }` —
confirmed to be identical, pre-existing behavior of `rusty_term` alone, not
a defect introduced here. A real terminal emulator needs a real console;
verify interactively.
