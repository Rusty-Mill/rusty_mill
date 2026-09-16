# coreutils Capability-Gap Backlog

Recorded 2026-07-21. `crates/coreutils` is this repo's own reference
consumer (`lib.rs`: "Utilities written *only* against the `platform`
trait surface... the exercise ground for the understanding mandate
(M1)"). This document is what came out of actually reading every line
of it against what `platform::*` currently offers — where the six
binaries (`rcat`/`rls`/`rrun`/`rtee`/`rpar`/`rterm`) had to reach past
`platform` to get something done, and where each of those reaches is a
real capability gap versus a deliberate design boundary versus a
feature `coreutils` itself just hasn't built yet on a capability that
already exists.

Same discipline as `docs/extraction-map.md`/`docs/convergence-roadmap.md`:
numbered, append-only, mark items done in place rather than deleting
history. Nothing here is authorized to start just by being listed —
RFC v2 §3's consumer gate still applies. Unlike those two documents,
every "consumer" here is inside this same repo, so there's no separate
"lands in the tool's own repo" split — everything below either lands in
`platform`/backends or in `coreutils` itself.

## Gap 1 — `rtee` bypasses `platform::fs` entirely for its output file (#62)

**File:** `crates/coreutils/src/bin/rtee.rs:30`

```rust
let mut out_file = match std::fs::File::create(file_arg) {
```

`rtee <file> -- cmd [args...]` takes `file` as a bare CLI argument —
an arbitrary ambient path, not a name relative to an already-open
`Dir`. Every other binary that faces the same problem (`rcat`, `rls`)
solves it by splitting the path into parent + file-name and calling
`coreutils::native::open_dir(parent)` then `.open(name, opts)` —
verbose, but stays on the `platform::fs::Dir`/`File` capability model.
`rtee` instead reaches straight for `std::fs::File::create`, which:

- Contradicts this crate's own stated design (`lib.rs`: "written
  *only* against the `platform` trait surface").
- Can't be exercised against `platform-mock` — this one file's I/O is
  the one thing in `rtee` no unit test can substitute.
- Gets none of `platform`'s byte-faithful error typing
  (`PlatformError`/`ErrorKind`) for this specific failure path — a
  permission-denied or not-found on `file_arg` surfaces as a raw
  `std::io::Error` via `Display`, not the same error shape every other
  `platform`-backed failure in the same binary produces.

**Root cause, not just a shortcut**: there is no single-call "open an
ambient path directly" helper anywhere in `coreutils::native` — only
`open_dir(path)`. The parent-split-then-open pattern `rcat`/`rls` use
is the only portable way to open an arbitrary path today, and it's
easy to reach past when just proving a binary works.

**Recommended fix**: two options, not mutually exclusive —
1. Fix `rtee` itself to use the same parent-split-then-open pattern
   `rcat` already established (`OpenOptions::create_truncate()`,
   matching `cat.rs`'s own read path). Small, no `platform` change.
2. Add a small `coreutils::native::open_ambient_file(path, opts)`
   convenience (split parent/name, `open_dir`, `.open`) so this exact
   mistake can't recur in the next binary that takes a bare path
   argument. Pure ergonomics — no new backend capability, since
   `Dir::open` already exists; this only saves every future consumer
   from re-deriving the split.

Both are coreutils/consumer-side fixes, not a `platform` trait gap —
recorded here because the *pattern* (bare CLI path arguments are
common) is a real recurring friction point worth fixing once.

## Gap 2 — `platform::fs::Metadata` has no timestamps (#63)

**File:** `crates/platform/src/fs.rs:86-91`

```rust
pub struct Metadata {
    pub file_type: FileType,
    pub len: u64,
}
```

No `modified`/`accessed`/`created` anywhere in `Metadata`, and no
timestamp field on `DirEntry` either. A real `ls -l`/`stat` cannot
report a modification time through this trait at all today — this is
a genuine missing OS-facing capability, not a coreutils oversight:
every backend would need to decide a wire type (`std::time::SystemTime`
is the obvious portable choice; Linux's `stat`/`statx` and Windows'
`GetFileInformationByHandle`'s `BY_HANDLE_FILE_INFORMATION` both already
expose it, no new syscall class needed) and how to handle each OS's
particular timestamp precision/epoch quirks.

**Status: no forcing consumer yet.** `coreutils::ls` doesn't have a
long-format (`-l`) mode to begin with (see Gap 4) — nothing in this
repo currently asks for a timestamp. Recording the gap now because it
was the most conspicuous absence found reading the trait, not because
anything is blocked on it. Per RFC v2 §3, do not build ahead of a
named need.

## Gap 3 — no `chmod`-equivalent (write path for permissions) (#64, closed)

**File:** `crates/platform/src/fs.rs:180-260` (the `Dir` trait)

`Dir::unix_mode` reads `setuid`/`setgid`/`sticky`/`uid`/`gid`, and
`Dir::access` probes read/write/execute — but nothing sets any of it.
There is no standard-rwx-bits read either (`UnixMode` only carries the
three special bits plus ownership, not the `0755`-style permission
bits `chmod`/`install`/a real `ls -l` would need). Entirely
speculative right now — no binary in `coreutils` needs it (there is no
`rchmod`), and per RFC v2 §3 this stays unbuilt until one does.
Recorded for completeness, lowest priority in this document.

**Resolved 2026-07-23**: `Dir::set_unix_mode` landed (a new `Mode`
struct: `setuid`/`setgid`/`sticky`/`permissions`, no `uid`/`gid` —
that's `chown`'s job) — `fchmodat` on Linux, `Err(Unsupported)` on
Windows (`docs/divergences.md` #009) and under the `track-p` feature
(`rusty_libc` has no `chmod` binding yet), a documented no-op on
`platform-mock`. Landed ahead of a named `coreutils` consumer (no
`rchmod` binary exists yet) — the read side (#63/#65) already forced
`UnixMode`'s shape, and `unix_mode` without its write-side counterpart
was an increasingly conspicuous half-finished capability on its own
terms, independent of `rchmod` specifically. See `docs/behavior/fs.md`
for the full contract.

## Gap 4 — `coreutils::ls` doesn't use the `Metadata`/`unix_mode` it already has (#65)

**File:** `crates/coreutils/src/ls.rs`

Not a `platform` gap at all — the opposite: `Dir::metadata` (file
size) and `Dir::unix_mode` (ownership/special bits) already exist and
`ls.rs` simply never calls them. `render()` only ever prints a name
plus a trailing `/` for directories; there is no `-l` long-format mode,
no size column, no symlink target (`Dir::read_link` already exists and
also goes unused here). This is a `coreutils`-side feature backlog
item layered on top of Gap 2 (timestamps still wouldn't be available
even if `ls -l` were built) — listed here so it doesn't get confused
with an actual capability shortfall when someone goes looking for why
`ls` output looks so bare.

## Gap 5 — `std::env::current_dir()` duplicated three times (by design, not a gap) (#66, closed)

**Files:** `crates/coreutils/src/bin/rrun.rs:15`,
`crates/coreutils/src/bin/rtee.rs:23`,
`crates/coreutils/src/bin/rpar.rs:19` — identical boilerplate in all
three.

`platform::process::Command::cwd` is `pub cwd: OsString` with no
"inherit ambient cwd" variant, and the field's own doc comment is
explicit about why: *"Always explicit... by design: consumers own
their cwd policy (rush virtualizes it; RFC v2 §5.3 rationale)."* A
shell that virtualizes `cd` internally (rush) must never let a spawned
child silently inherit the OS-ambient cwd instead of the shell's own
tracked one — so `platform` deliberately does not offer an ambient-cwd
shortcut, and every consumer that *does* want "just use wherever this
process happens to be running" (all three `coreutils` binaries here)
has to say so explicitly via `std::env::current_dir()`.

**Not a gap** — recorded here only so the repeated boilerplate doesn't
get miscategorized as a missing `platform` primitive by someone
skimming for patterns. If this exact duplication is ever worth
removing, it's a `coreutils`-internal helper (a small
`coreutils::native::ambient_cwd() -> Result<PathBuf>` wrapping the same
`std::env::current_dir()` call with the same error message shape),
never a `platform::process` addition — that would reintroduce the
exact ambient-inheritance shortcut the design rationale above rules
out.

## Gap 6 — `rcat`/`rtee` write to `std::io::stdout()` directly (reviewed, not a gap) (#67, closed)

**Files:** `crates/coreutils/src/bin/rcat.rs:29`,
`crates/coreutils/src/bin/rtee.rs:58`.

Both binaries write the bytes they read through `platform::fs::File`
straight to `std::io::stdout().lock()`, not through any `platform`
type. Considered and dismissed as a gap: the process's own inherited
stdout is not a capability this crate's Dir-relative-open security
model governs (there is nothing to confine — it's already a bare
inherited fd/handle at process start, on every OS), and
`std::io::Write` is already fully portable for the "copy these bytes
out" operation with no backend-specific behavior to abstract over.
Recorded so a future read of this backlog doesn't re-flag it.

## Summary

| # | Item | Kind | Action | Issue |
|---|------|------|--------|-------|
| 1 | `rtee`'s `std::fs::File::create` bypass | Consumer bug + ergonomics gap | Fix `rtee`; consider `open_ambient_file` helper | #62 |
| 2 | No timestamps in `Metadata` | Real `platform` capability gap | Hold — no forcing consumer | #63 |
| 3 | No `chmod`-equivalent write path | Real `platform` capability gap | Landed: `Dir::set_unix_mode` | #64 (closed) |
| 4 | `ls` doesn't use `metadata`/`unix_mode`/`read_link` it already has | `coreutils` feature backlog | Hold — cosmetic, no urgency | #65 |
| 5 | Triplicated `std::env::current_dir()` | By design, not a gap | No `platform` change; optional `coreutils` helper | #66 (closed) |
| 6 | Direct `std::io::stdout()` writes | Reviewed, not a gap | None | #67 (closed) |
