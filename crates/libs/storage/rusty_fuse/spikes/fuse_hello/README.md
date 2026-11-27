# Spike B: `fuse_hello` (Linux)

Phase 0 spike B from `docs/projects/rusty_fuse/phase0-plan.md`, board card #1615.
Throwaway, test-exempt code (QA.md § Testing). It is outside the workspace
(root `Cargo.toml` `exclude`), so workspace CI does not build it.

A read-only hello filesystem (`/hello.txt`, `/dir/a.txt`) that speaks the
kernel protocol over raw `/dev/fuse`. It uses no libfuse and no third-party crate.
`mount`, `umount2`, `unshare` and `statfs` are spike-local syscalls through
`rusty_libc::arch` (D13, D15).

## Run

You need Linux x86_64 in a disposable environment. Build as any user, then run
the evidence script as root:

```
cargo build --release
sudo bash run.sh
```

The script covers four cases: a root mount with a self-test, a root mount checked
by `ls` and `cat` from another process, a non-root plain `mount(2)` (EPERM expected),
and a non-root `unshare(CLONE_NEWUSER|CLONE_NEWNS)` followed by `mount(2)` (Q5). It
expects the non-root user to be `spike` and the binary under `/home/spike/rm/...`.
Adjust `bin` and the user name in `run.sh` for another layout.

## Evidence

`evidence-2026-11-25-wsl2.log` comes from a disposable WSL2 distro on Beast: Ubuntu
24.04.5, kernel 6.18.58.1-microsoft-standard-WSL2. Every check passed as root and
in a user namespace. The kernel protocol is 7.45 and the spike negotiates 7.31.
In case C the log prefix reads `[root]`, because `run` hard-codes that label. The
process there is really the `spike` user (`su spike` in `run.sh`).
The `fusermount3` fd-passing path was not exercised. The Q7 probe on the GitHub CI
runners was not run.
