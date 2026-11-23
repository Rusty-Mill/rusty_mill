# Implementer Q&A — rusty_fuse
Pre-answered from the design chat and `../design.md`. Each cites its source. The expert adds entries as questions arrive.

## Scope and platforms
- **Q:** Which platform first? **A:** Windows (ProjFS). [stated] D1
- **Q:** Do I build macOS? **A:** Not now; low priority. Phase 5. [stated] D4
- **Q:** Is `fusermount3` allowed? **A:** Yes, as a transitional dependency behind the `Mounter` seam, with a migration plan. [stated] D2, D15
- **Q:** May I depend on libfuse, macFUSE, WinFsp or the `fsk`/`fuser` crates? **A:** Not on the default build. WinFsp is an optional feature-gated Tier A adapter, built only when a consumer needs provider-owned writes on Windows. `fsk` is a reference only. [default] D7, D8
- **Q:** Where does the code go? **A:** `crates/libs/storage/rusty_fuse/` [stated] D3; sub-crates per D12.
- **Q:** What is the first backend? **A:** Not decided. Escalate; do not start Phase 1. OPEN_QUESTIONS Q1.

## Port
- **Q:** Inode-based or path-based? **A:** Inode-based (`NodeId`, root is 1); the Windows adapter keeps a path-to-node map. [agreed] D6
- **Q:** async? **A:** No. Worker threads. [default] D11
- **Q:** Xattrs, locks, caller pid, cancellation? **A:** Not in v1. Pid is not portable (FSKit gives none); an adapter-specific extension may expose it. [default] D9
- **Q:** Error type? **A:** The port's own `Errno` enum; each adapter maps it (Linux errno, POSIX errno on macOS, HRESULT on Windows). Not `rusty_libc::Errno`. [default] design.md § port
- **Q:** What is `Name`? **A:** A validated type of raw bytes (WTF-8 on Windows); it cannot hold `/`, NUL, `.` or `..`. [default] D8
- **Q:** How do read-write backends interact with in-box Windows? **A:** They do not compile against `mount_projected`; only `mount_ro` or `mount_projected` exist there. [default] D10

## Adapters
- **Q:** How does Linux mount? **A:** Open `/dev/fuse`, `mount(2)` with `CAP_SYS_ADMIN`, or via `fusermount3` through the `Mounter` seam. Sans-IO codec in `proto`, worker pool in `session`. [default] D15, design.md § Linux
- **Q:** What are the v1 FUSE opcodes? **A:** `INIT`, `DESTROY`, `LOOKUP`, `FORGET`, `BATCH_FORGET`, `GETATTR`, `OPENDIR`, `READDIR`, `RELEASEDIR`, `OPEN`, `READ`, `RELEASE`, `READLINK`, `STATFS`, `INTERRUPT`; the write set arrives in Phase 3; the rest answer `ENOSYS`. [default] design.md § Linux
- **Q:** ProjFS binding? **A:** Load `ProjectedFSLib.dll` at run time with `rusty_win32::dynlib`; if the feature is off, `mount_projected` returns a clear `MountError` and the binary still starts. [default] D13
- **Q:** Illegal or case-colliding Windows names? **A:** Skip, log, and record in the divergence registry. [default] design.md § ProjFS
- **Q:** Hydrated data left on disk? **A:** Best-effort eviction of verified clean, provider-owned cache only. Local edits, creates, metadata changes and deletions are preserved by default. No destructive cleanup, and eviction is not secure erasure. [default] D16
- **Q:** May the adapter delete a dirty or full file on unmount? **A:** No. That needs a separate, explicit, owner-approved operation this design does not authorize. [default] D16

## Testing
- **Q:** How is it tested without a mount? **A:** Codec fixtures and fuzzing; `memfs` port tests on every OS; conformance mounts `memfs` through the real adapter on Linux and Windows CI; divergences numbered in `divergences.md` (rustils `platform-parity` pattern). macOS mount tests are manual. [default] design.md § Testing
- **Q:** Unit tests for spikes? **A:** Exempt (Nano's standing rule: tests for non-trivial logic, spikes exempt). [stated] preferences
