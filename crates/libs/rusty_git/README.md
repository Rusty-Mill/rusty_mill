# rusty_git

A real (if intentionally scoped-down) pure-Rust Git implementation: real
SHA-1 content addressing, real zlib-wrapped loose objects, a real
binary-compatible index (staging area), and real nested tree/commit
objects — verified against a genuine `git` installation, not just
internally self-consistent (see `src/lib.rs`'s `real_git_interop_tests`,
which shell out to the system `git` binary to prove `cat-file`, `log`, and
`ls-files --stage` can all read what this crate writes).

## What's real

- **Objects** (`src/objects.rs`) — blobs, trees, and commits framed exactly
  as git does (`"<type> <len>\0<content>"`, hashed with real SHA-1),
  including git's actual tree-entry sort rule (a subtree sorts as though
  its name had a trailing `/`).
- **Index** (`src/index.rs`) — the real git index-v2 binary format (`DIRC`
  header, fixed-width entries, a trailing SHA-1 checksum) — real
  `git ls-files --stage` can read it.
- **Trees** (`src/tree_builder.rs`) — builds real, possibly-nested tree
  objects from staged paths, one tree object per directory level.
- **Commits** — real parent chains; `rgit log` walks them the same way
  `git log` does, not from a side log file.
- **CLI** (`rgit`) — `init`, `add` (including whole-directory recursive
  add), `status` (real staged-vs-`HEAD` and worktree-vs-index comparison),
  `commit`, `log`, `diff` (via [`rusty_diff`](../rusty_diff)), `branch`.

## Known, deliberate gaps

- **Read/write asymmetry** (`src/zlib.rs`): this crate can *write* objects
  real git reads (spec-legal "stored" DEFLATE blocks, wrapped in a real
  zlib header/Adler-32 trailer), but can only *read back* objects it wrote
  itself — real git's Huffman-coded compressed objects use a DEFLATE block
  type [`rusty_compress`](../rusty_compress) doesn't implement yet. Once
  `rusty_compress` gains real Huffman decoding, this crate's `read_object`
  gets arbitrary-real-repo compatibility for free.
- **Index stat-cache fields are always zero** (`src/index.rs`): `ctime`/
  `mtime`/`dev`/`ino`/`uid`/`gid` are real git's perf heuristic for
  skipping a full content hash comparison, not a correctness requirement —
  zeroing them just means this crate always takes the slow (hash-compare)
  path.
- **No packfiles, no merge, no remotes.** This is a local, single-branch
  workflow (init/add/commit/status/diff/log), not a clone/push/pull-capable
  client.
- **Timestamps are always `+0000` (UTC)** — no local timezone-offset
  computation.

## Testing

```
cargo test
```

Includes real interop tests (skipped, not failed, if `git` isn't on `PATH`)
that shell out to the system `git` binary to verify actual compatibility.
