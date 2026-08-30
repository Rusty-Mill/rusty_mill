# 004 — A donor binding is shaped by its own consumer

Encountered migrating `sys::proc`'s spawn/wait/job families to Track W
(platform-windows, D-15's second slice) — the families the convergence
roadmap called "the donor's own strongest material."

Nine of ten call sites migrated mechanically. The tenth,
`CreateProcessW` itself, cannot, and the reason is the useful part: a
hand-written binding is not a neutral declaration of an OS call. It
carries the shape its author's own consumer needed, and that shape can
be *incompatible* with a different consumer's requirements rather than
merely narrower.

## What migrated

| `sys::proc` | Track W |
|---|---|
| `create_kill_on_close_job` | `job::create` + `job::set_kill_on_close` |
| `adopt` | `process::open_by_pid` + `job::assign` |
| assign step in `spawn` | `job::assign` |
| resume step in `spawn` | `process::resume` |
| `terminate_job` | `job::terminate` |
| `terminate_process` | `process::terminate` |
| `try_wait` | `process::wait(h, Some(0))` |
| `wait` | `process::wait(h, None)` |
| `wait_many`'s single call | `process::wait_any` |

The 64-handle chunking in `wait_many` stays ours: the donor's `wait_any`
reports `ERROR_INVALID_PARAMETER` past the cap, exactly as the raw call
does, which is the right call for a binding and the wrong one for a
reactor. Absorbing a documented OS limit is the layer's job, not the
binding's — the same division the fileio slice found with its length
clamp (note 003), showing up again one layer up.

## What didn't, and why it isn't a gap to file

`rusty_win32::process::spawn_suspended` passes a bare zeroed
`STARTUPINFOW` and a `NULL` `lpCurrentDirectory`, and takes its command
line as `&str`. Three blockers, none of them a missing feature someone
could add without changing what that function *is*:

1. **No per-spawn std-handle override.** Its docs say so plainly:
   "redirect by swapping the parent's std-handle slots before spawning."
   That is rush's `winstdio` model, and it is coherent — for a shell,
   which owns its own std slots and is single-threaded about spawning.
   This backend deliberately rejected it (extraction map D5, step 4:
   `STARTF_USESTDHANDLES` per spawn, so `Stdio::{Null, Pipe, File}`
   never mutates process-global state). Migrating would not be adopting
   a binding — it would be reversing a recorded architectural decision
   and deleting the `Stdio` mechanism with it.
2. **No working directory.** `Command::current_dir` has nowhere to go.
3. **`&str`, not `&[u16]`.** winargv is this crate's security boundary;
   round-tripping its `&[u16]` through `String` is lossy for unpaired
   surrogates. Not a trade a migration slice gets to make.

The temptation is to file these as donor gaps and "close" them. That
would be wrong, and naming why is the lesson: a `spawn_suspended` that
grew a `STARTUPINFOW` override, a cwd, and a `&[u16]` command line would
just be *this crate's* `spawn`, living in the other repo. The donor's
version is not an incomplete draft of ours. It is a different, correct
answer to a different consumer's question, and the honest outcome is two
implementations that stay two.

Generalized, and worth carrying into the remaining Track W slices: **ask
what the binding's original consumer needed before assuming a signature
gap is an oversight.** Track P never surfaced this because `rusty_libc`
binds syscalls, and a syscall's signature is fixed by the kernel — there
is no room for an author's consumer to leave fingerprints. A DLL-export
binding has all the room in the world, which is precisely the freedom
that makes Track W a provenance swap (note 003) rather than a descent.

## One divergence, recorded rather than smoothed over

`process::wait` folds `WaitForSingleObject` and `GetExitCodeProcess` into
a single call, so a failure of the second surfaces under the first's
name in `PlatformError::op`. `ErrorKind` and `OsCode` — the fields
anything actually branches on — stay identical, since both arms classify
the same `GetLastError` code through the same table. `op` is a
diagnostic label in the `Display` string and is asserted nowhere, so this
is cosmetic; it is written down anyway because the slice's whole claim is
that the two configurations are indistinguishable, and "indistinguishable
except here" is a different and more useful claim than a silent asterisk.

The same folding produced a second, better outcome in `try_wait`. The
windows-sys arm deliberately reports *any* non-exit result as "still
running" (its own comment: a poll reports liveness only, and a genuine
failure surfaces on the eventual blocking `wait`). The donor's `wait`
returns a real `Err` there. Preserving the existing contract meant
mapping that `Err` back to `Ok(None)` — which looks like swallowing an
error until you notice that changing it would make a poll behave
differently under one feature flag than the other. Fidelity to the
existing contract beats fidelity to the donor's richer return type; the
equivalence test is only worth something if the thing being tested is
equivalence.
