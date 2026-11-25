---
category: Added
changelog: rusty_fuse port crate, memfs conformance suite and a read-only passthrough backend that mirrors a plain directory (no mount, no ProjFS).
---
## 2026-11-25 - rusty_fuse port, conformance and passthrough backend

- **Added:** `rusty_fuse`, the backend-agnostic port: the read-only `Filesystem` trait (`lookup`, `getattr`, `readdir` with cookies and verifiers, `open`, `read`, `release`, `readlink`, `forget`) and its types (`NodeId`, `Name`, `Errno`, `Attr`, `Entry`). No I/O, no `unsafe`, no dependencies.
- **Added:** `rusty_fuse_conformance`, test support: an in-memory `MemFs`, a shared fixture tree and assertions every backend must pass.
- **Added:** `rusty_fuse_passthrough`, a read-only backend that mirrors a host directory with `std::fs`. Symlinks and junctions are served as symlink nodes and never followed; names that could leave the root are refused. Tested on a temp directory on every OS, which is plain NTFS on Windows.
- **Known limitations:** no OS mount yet (no FUSE, ProjFS or FSKit adapter) and no `Writable`. The first real backend is still open (Q1), so the port may change. Symlink containment checks race against concurrent local writers to the source folder; the mirror is not a security boundary. On Windows, names that are not valid Unicode are skipped, and case-variant lookups resolve to different node ids.
