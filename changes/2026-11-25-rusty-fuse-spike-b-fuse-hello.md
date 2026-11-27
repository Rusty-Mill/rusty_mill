---
category: Added
changelog: rusty_fuse Phase 0 spike B (fuse_hello) mounts, serves and unmounts a raw /dev/fuse filesystem, as root and in a user namespace (card #1615).
---
## 2026-11-25 - rusty_fuse spike B: fuse_hello (card #1615)

- **Added:** the throwaway spike `crates/libs/storage/rusty_fuse/spikes/fuse_hello`. It speaks the FUSE kernel protocol over raw `/dev/fuse` with only `rusty_libc` as a dependency: no libfuse and no third-party crate. It has spike-local `mount`, `umount2`, `unshare` and `statfs` syscalls, and it is excluded from the workspace (D17). The included evidence log is from a disposable WSL2 Ubuntu 24.04 distro on kernel 6.18. INIT, ls, cat, statfs and a clean umount2 all pass as root, and also as non-root after `unshare(CLONE_NEWUSER|CLONE_NEWNS)` (Q5). A plain non-root mount gets EPERM.
- **Known limitations:** it was tested on WSL2 only. Neither the Q7 probe on the GitHub CI runners nor the `fusermount3` fd-passing path was exercised. It is spike code with no tests (QA.md § Testing).
