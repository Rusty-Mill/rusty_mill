# Sovereign cross-platform userspace filesystem: design (condensed copy)

Not binding. Condensed from the Claude Doc rev 24 (https://claude.ai/artifact/Shrtmm4Ubn68pRAh33it4D), 2026-11-19. The doc is the full text and has the architecture diagram, which no export carried. Binding decisions are in `handoff/DECISIONS.md`.

## Summary
One inode-based `Filesystem` port in Rust with a native adapter per OS, using only mechanisms that ship in the box: the FUSE kernel protocol on Linux, FSKit on macOS, ProjFS on Windows. A backend is written once and mounted anywhere. The mechanisms differ in capability, so the port is a small read core plus optional capability traits, and each adapter declares what it can serve. WinFsp (third-party kernel driver) stays an optional feature-gated adapter for provider-owned writes on Windows.

## Goals and non-goals
Sovereign: no third-party kernel code, no third-party userspace library between backend and kernel (rules out libfuse, macFUSE and, by default, WinFsp; matches ADR-0002 Tier S/T). Goals: one backend on three OSes; in-box mechanisms only on the default build; capability differences visible in the type system; port crate free of I/O and `unsafe`; v1 port under a dozen operations.
Non-goals: kernel drivers; full POSIX on in-box Windows; FreeBSD; async API; network filesystem protocols (loopback NFS rejected: the Windows NFS client is not in Home editions; loopback SMB fights the OS for port 445).

## Platform landscape
| | Linux | macOS | Windows (in-box) | Windows (WinFsp) |
|---|---|---|---|---|
| Mechanism | FUSE protocol over `/dev/fuse` | FSKit app extension | ProjFS | WinFsp driver |
| Ships with OS | yes | yes | yes, optional feature an admin enables | no |
| Addressing | inode numbers | items (`reclaimItem` ~ FUSE forget) | paths | paths and file contexts |
| Who owns writes | provider | provider | NTFS; provider gets notifications | provider |
| Where data lives | provider process | provider process | hydrated onto local NTFS | provider process |
| Extra requirement | privileged `mount(2)` or `fusermount3` | Swift extension, Apple signing, user enables it | feature enabled by admin | driver installed and approved |
Consequences: the port is inode-based and the Windows adapter keeps a path-to-node map; ProjFS reads content from the provider once, then serves it from NTFS, so a backend whose content changes must tell the adapter to invalidate; FSKit gives no caller pid, so pid is not in the core.

## Architecture
Dependencies point down only: backend knows the port, the port knows nothing, each adapter knows the port and one OS. Adapters never import each other; a facade crate picks one by target OS. Windows offers a different mount function because ProjFS cannot give the provider ownership of writes.

## Port and capability model
```rust
pub struct NodeId(NonZeroU64);              // stable for the life of a mount; root is 1
pub enum NodeKind { File, Dir, Symlink }
pub struct Attr { kind, size: u64, perm: u16, nlink: u32, uid: u32, gid: u32, mtime, ctime }
pub struct Entry { id: NodeId, generation: u64, attr: Attr, ttl: Ttl }

pub trait Filesystem: Send + Sync {
    fn lookup(&self, parent: NodeId, name: &Name) -> Result<Entry, Errno>;
    fn getattr(&self, id: NodeId) -> Result<Attr, Errno>;
    fn readdir(&self, dir: NodeId, cookie: Cookie, verifier: Verifier, sink: &mut dyn DirSink) -> Result<Verifier, Errno>;
    fn open(&self, id: NodeId, flags: OpenFlags) -> Result<Handle, Errno>;     // default Ok(Handle::NONE)
    fn read(&self, id: NodeId, fh: Handle, offset: u64, buf: &mut [u8]) -> Result<usize, Errno>;
    fn release(&self, id: NodeId, fh: Handle) {}
    fn readlink(&self, id: NodeId, buf: &mut Vec<u8>) -> Result<(), Errno>;    // default Err(Inval)
    fn forget(&self, id: NodeId, nlookup: u64) {}
}

pub trait Writable: Filesystem {
    fn create, mkdir, write, setattr, unlink, rmdir, rename, fsync   // fsync defaults to Ok
}
```
Choices: `Name` is validated raw bytes (WTF-8 on Windows); `read` fills a caller buffer (no allocation per read); `readdir` pushes into a sink with stable cookies, and a returned verifier changes when the directory does; `Errno` is the port's own enum, mapped per adapter; no request context in the core; backends use interior mutability and are `Send + Sync`.
Capability at compile time: each adapter crate exposes `mount_ro<F: Filesystem>` and, only where the OS lets the provider own writes, `mount_rw<F: Writable>`. The ProjFS adapter has no `mount_rw`; the facade offers `mount_projected<F: Filesystem + Projected>` on Windows, where `Projected::changed(&self, sink)` reports nodes to re-hydrate. At run time `mount_*` returns `Result<Mount, MountError>` naming what is missing (feature off, `/dev/fuse` absent, no permission, protocol too old, extension not enabled). `Mount` unmounts on drop. `Xattrs` and `Locks` are deferred.

## Adapters
Each adapter: a `sys` module with all `unsafe` and raw bindings, a sans-IO codec or translation layer, a thin driver that owns threads.
- **Linux.** Speak the kernel protocol on `/dev/fuse` directly. Mount: open the device, `mount(2)` with `fd=N,rootmode=...,user_id=...,group_id=...` (needs `CAP_SYS_ADMIN`), or the setuid `fusermount3` helper, which passes the fd back over a Unix socket (it ships in the distribution's fuse3 package, so an external run-time dependency, accepted as transitional: D2). Session: one request per `read()`, decode the 40-byte header and body, call the port, answer with a 16-byte header via `writev`. `FUSE_INIT` negotiates the minor version and maximum write. Fixed worker pool. Codec unit-tested and fuzzed with no kernel.
- **Migrating off `fusermount3`.** Tier T needs an owner, a first-party seam and a milestone. Seam: a `Mounter` trait returning the opened device fd; `Fusermount3` first. Replacement 1: direct `mount(2)` with `CAP_SYS_ADMIN` (needs `mount`/`umount2` wrappers in `rusty_libc`). Replacement 2: user namespace mount, kernel 4.18+ (working knowledge; confirm in a spike; needs `unshare` in `rusty_libc`). Proposed milestone: replacement 1 with Phase 3; dependency removed once both are tested. Owner: unassigned.
- **macOS.** Two halves: a small Swift `FSUnaryFileSystem` extension that forwards each volume operation to the Rust process and holds no logic (lives in `macos/`, outside Cargo, needs Xcode and signing), and a Rust bridge server on a Unix domain socket with fixed little-endian frame headers multiplexed by request id. FSKit items map to `NodeId`; `reclaimItem` maps to `forget`. Unknowns for a spike: FSKit resource type with no block device, minimum macOS version, how the sandboxed extension reaches the socket, signing entitlement.
- **Windows ProjFS.** Provider over a virtualization root on NTFS. Bind `ProjectedFSLib.dll` at run time via `rusty_win32::dynlib`; feature off gives a clear `MountError` and the binary still starts. Callbacks: enumeration calls `readdir`, placeholder info calls `lookup`/`getattr`, file data calls `open`/`read`/`release`. Path-to-node map built by walking `lookup`, case-insensitive. Enumeration: ProjFS wants its own sort order and search patterns, so snapshot a directory per session, sort, filter, serve pages. Invalidation after hydration: `Projected::changed` drives `PrjDeleteFile` or `PrjUpdateFileIfNeeded`. Writes become full NTFS files; the adapter sees notifications and can report or veto some, never redirect data. Illegal or case-colliding names are skipped, logged and put in the divergence registry.
- **WinFsp (optional, later).** Feature-gated adapter giving `mount_rw` on Windows where the driver is approved. Tier A. Built only when a consumer needs provider-owned writes on Windows.

## Reuse from rusty_mill (surveyed 2026-11-19)
Nothing implements FUSE, ProjFS, FSKit or WinFsp today. Use: `rusty_wire` (endian-explicit byte cursor, zero deps), `rusty_libc` (raw Linux syscalls incl. `send_with_fds`/`recv_with_fds`; add `mount`, `umount2`, `unshare`), `rusty_win32::dynlib`, `rusty_err`, `rusty_log`, rustils `platform` fs traits (passthrough backend), `rusty_confined_fs`, rustils `platform-parity` and `divergences.md` (conformance pattern). Pattern only: `rusty_mux` `proto` (apps cannot be library dependencies, ADR-0003). Not used: `rusty_tokio`/`rustils_async` (blocking interfaces), `rusty_sync` (no_std spinlocks), `rpath`, `rusty_sandbox` (later hardening). Constraints: rustils RFC v2 §3 consumer gate (new OS calls start in the adapters; hoist at a second consumer); macOS is the thinnest target. Candidate backends: `rusty_sqlite`, `rusty_multimodal_db_engine`, `rusty_git`.

## Prior art: `fsk` crate
Same FUSE/ProjFS/FSKit split behind one trait set. Read from docs.rs excerpts for 0.0.1-0.0.9 only (pages not fetchable; licence, dependencies, dates unchecked). Confirms: inode-based core; readdir with cookie and mutation verifier into a sink (adopted); names as bytes, WTF-8 on Windows (adopted); remote vs local-leased file opening (deferred to Phase 4); opt-in raw native contexts (same as our adapter extensions); ProjFS projection-only limits and WinFsp for provider-owned writes; install-once Swift bridge, fixed little-endian frames, macOS 26+ (`fskit-rs` says 15.4+). Linux path uses `fuser` and installed FUSE, so we do not depend on it.

## Crate layout
Under `crates/libs/storage/rusty_fuse/` (ADR-0003):
```
crates/rusty_fuse             port: traits, types, Errno; forbid(unsafe), no OS calls
crates/rusty_fuse_linux       /dev/fuse: sys, proto (sans-IO), session
crates/rusty_fuse_projfs      sys, pathmap, enumeration, provider
crates/rusty_fuse_fskit       bridge server + frame codec
crates/rusty_fuse_mount       facade: re-exports the adapter for the target OS
crates/rusty_fuse_passthrough reference backend over rustils platform fs
crates/rusty_fuse_conformance memfs + shared assertions (test support only)
macos/                        Swift extension (Xcode, outside Cargo)
docs/                         behaviour specs, divergences.md, ADRs
```
No new third-party crate on the default build; external pieces are OS components only (FUSE kernel module, `ProjectedFSLib.dll`, FSKit).

## Concurrency, errors, caching, security
Backends are `Send + Sync`, called concurrently; no adapter holds a lock across a backend call. `FUSE_INTERRUPT` and ProjFS cancel are acknowledged and otherwise ignored in v1 (token added when slow I/O needs it). Adapters translate errors at the edge and log what they cannot map with context; a backend panic is caught at the adapter boundary, answered as an I/O error, and logged. `Entry.ttl` tells the kernel how long to trust a name; zero is always right and slow; adapters never cache file data on Linux/macOS; on Windows the OS caches by design. A dead Linux provider leaves "transport endpoint not connected" until unmount: ship a `recover` helper. Security: all `unsafe` in `sys` modules; kernel input is untrusted and validated in the codec (fuzzed); Linux mounts default to owner-only with kernel permission checks (`allow_other` explicit); ProjFS hydration writes content to local NTFS, so treat the virtualization root as data at rest (purge on unmount, on by default); never log file contents or credentials; paths at debug level only.

## Testing
Codec unit tests on captured byte fixtures (every OS, no mount); fuzzing of the FUSE and bridge decoders (Linux, scheduled); port tests with in-memory `memfs` (every OS, no mount); conformance mounting `memfs` through the real adapter and driving it with ordinary file APIs (Linux and Windows CI); numbered, documented, per-adapter-asserted divergence registry; macOS FSKit mount verified manually until CI exists (an Apple-forum thread raises exactly this). Linux CI needs `/dev/fuse` plus root or `fusermount3`; Windows CI needs ProjFS enabled; both checked in Phase 0. Failure cases are first-class: feature missing, no permission, provider crash, backend panic, illegal names, case collisions.

## Delivery plan
0 spikes: ProjFS hello-world via `rusty_win32::dynlib` and `/dev/fuse` hello-world via `rusty_libc`; confirm ProjFS can be enabled on a managed machine; exit is go/no-go per platform and CI feasibility. 1 port + Windows read-only: `rusty_fuse`, `memfs`, conformance, `rusty_fuse_projfs` with `mount_projected` (snapshots, invalidation). 2 Linux read-only: codec with fuzzing, `Mounter` seam with `Fusermount3`, `mount_ro`; also checks the port still fits FUSE unchanged. 3 Linux read-write: `Writable`, write opcodes, passthrough backend, first `fusermount3` replacement. 4 on demand: WinFsp adapter, `Xattrs`, `Locks`, cancellation, local-file leases (each needs a named consumer). 5 macOS: Swift extension and `rusty_fuse_fskit`. Risk of Windows-first: shaping the port around ProjFS's path-based, merge-with-local model; mitigations are port tests on every OS and the Phase 2 check. If the first real backend must own writes on Windows, WinFsp moves up to Phase 2 and the Windows default is no longer all in-box.

## Risks (summary)
FSKit may not host a block-device-less filesystem on the target macOS (spike first); FSKit signing entitlement may be unobtainable (macOS stays developer-only); ProjFS may be too weak for the first backend (pick the backend before Phase 1); ProjFS may be impossible to enable on managed machines (confirm with the approving authority); unprivileged Linux mounts need `fusermount3` (D2); hydrated data on disk (purge on unmount); FUSE protocol drift (negotiate minor in `FUSE_INIT`, ignore unknown fields).

## Sources
Apple FSKit documentation; macFUSE project page (FSKit backend, macOS 26); Apple forum threads on FSKit (reclaim/forget, no process attribution; CI automation); `fsk` on docs.rs 0.0.9; `fskit-rs` on Rust Digger; `fuser` on GitHub; `Rusty-Mill/rusty_mill` `ARCHITECTURE.md`, ADR-0002, ADR-0003. The Linux FUSE protocol and ProjFS API details were not checked against primary documentation.
