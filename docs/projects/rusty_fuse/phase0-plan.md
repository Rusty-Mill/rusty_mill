# rusty_fuse Phase 0 plan

Date 2026-11-20. Scope: SPEC.md § Acceptance, first slice. Spikes only; no Phase 1 and no crates beyond throwaway spike code until Nano names the first real backend (Q1). Spike code is test-exempt (QA.md § Testing).

## Ground rules
- Location: `crates/libs/storage/rusty_fuse/spikes/<name>/` (D17). Crates `rusty_fuse_spike_<name>`, `publish = false`, listed in root `Cargo.toml` `exclude`, so workspace CI does not build them.
- Dependencies: path deps on workspace crates only (`rusty_win32::dynlib`, `rusty_libc`); no libfuse, WinFsp, `fsk` or other third-party crate (D7, D8).
- Missing wrappers (`mount`, `umount2`, `unshare`) stay spike-local via `rusty_libc::arch` `syscallN`. Hoisting is Phase 3 (D13, D15).
- No Phase 1 types, no port crate, no backend choice. macOS is out (D4).
- Each spike prints a one-screen evidence log. Evidence goes to `rusty-fuse-expert`, which records go/no-go in DECISIONS.md, ticks SPEC § Acceptance and closes Q5 and Q7.

## Spike A: `projfs_hello` (Windows, Nano's machine)
Order: first (D1).
1. Bind `ProjectedFSLib.dll` with `dynlib::load_library` and `get_proc_address`. With the optional feature off, exit with a clear error, not a crash (QA § Adapters). Records the first half of Q2.
2. Mount: `PrjMarkDirectoryAsPlaceholder`, `PrjStartVirtualizing` over a temp root, with a fixed in-memory tree (`/hello.txt`, `/dir/a.txt`).
3. Callbacks: start/end enumeration, get placeholder info, get file data.
4. **Go** when: the root mounts, `dir` lists the tree, and reading `hello.txt` hydrates it.
5. Note PrjFS limits seen (name case, sort order, search patterns) for D16.

### A2: clean versus local changes (eviction check)
Confirm exact cache-state names and the non-forcing `PrjDeleteFile` update flags against Microsoft's cache-state and PrjDeleteFile docs, then test with the five fixtures below. Eviction uses only non-forcing calls; a refusal is skip and log, never a retry with override flags (D16).

| Fixture | Setup | Expected after eviction |
|---|---|---|
| clean hydrated | read `hello.txt` | removed from cache (becomes placeholder or gone) |
| edited | read, then append locally | untouched |
| locally created | new file in root | untouched |
| changed metadata | read, then change attributes or timestamp | untouched |
| locally deleted | delete a projected file | tombstone untouched |

No destructive cleanup and no secure-erasure claim. Record the cache-state names observed and the flag values used.

## Spike B: `fuse_hello` (Linux)
This cloud workspace has `/dev/fuse` (root-only, uid 0) and no `fusermount3`, so it can run as root here. The kernel is 6.18.
1. `open("/dev/fuse")` via `rusty_libc::fd::open`; spike-local `mount(2)` syscall with `fd=N,rootmode=40000,user_id=0,group_id=0`; tiny single-threaded loop: `read` request, decode the 40-byte header, answer with `writev`.
2. Opcodes: `INIT` (negotiate minor), `LOOKUP`, `GETATTR`, `OPENDIR`, `READDIR`, `RELEASEDIR`, `OPEN`, `READ`, `RELEASE`, `STATFS`, `FORGET`, `DESTROY`; others `ENOSYS`.
3. **Go** when: `INIT` completes and `ls` plus `cat` work on the mount, and unmount (`umount2`) is clean.
4. Q5: try `unshare(CLONE_NEWUSER|CLONE_NEWNS)` then `mount(2)` as non-root and record the result and distro limits. A no-go here changes D15 replacement 2 only.
5. Q7: record whether `/dev/fuse` plus root or `fusermount3` exists here and on the CI runners (check a workflow job with a one-off probe step, not committed to `ci.yml` in Phase 0), and whether the Windows runner can enable ProjFS (`Enable-WindowsOptionalFeature`).

## Exit
Go/no-go per platform recorded by the expert; CI feasibility known for both; Q2 (first half), Q5, Q7 answered or restated. Then stop. Phase 1 waits on Q1.

## Waits on Nano
- Q1 first real backend (gates Phase 1).
- Q10 answered by Nano 2026-11-20 ("As an experimental project"): Phase 0 is a labelled `experiment`, no PRIORITIES.md row. Board project `rusty_fuse` (status experiment) holds cards #1613 (spike A), #1614 (A2), #1615 (spike B). The experiment ends with a one-page harvest in remind-me.
- Spike A needs Nano's Windows machine (run it, or authorise a Remote Control session there).

## Board check (2026-11-20)
No card in any project mentions ProjFS, WinFsp, FSKit or a userspace filesystem; the only "fuse" hits are unrelated (#81 CSI scan merge, #255 traversal merge, both done). Nothing to link or supersede.
