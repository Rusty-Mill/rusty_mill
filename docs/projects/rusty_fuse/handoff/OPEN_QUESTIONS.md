# Open questions — rusty_fuse
Claude Code: append here instead of stopping when a question isn't blocking.
Format: `- [ ] Q<n> (<date>, blocking? y/n): <question> — interim choice: <what you did>`
Resolved items are checked and point to the D#/QA entry that answers them.

- [ ] Q1 (2026-11-20, y for Phase 1): What is the first real backend, and does it need provider-owned writes on Windows (which would pull WinFsp, Tier A, into Phase 2)? Candidates already in the workspace: `rusty_git` tree, `rusty_sqlite` view, remind-me store. — interim: Phase 0 only; no Phase 1 work. Needs Nano.
- [ ] Q2 (2026-11-20, n): Can ProjFS be enabled on the machines this will run on (managed NIPR/SIPR)? Would WinFsp be approvable if needed? Nano: "not sure". — interim: Phase 0 spike on Nano's own machine answers the first half only; the approving authority answers the rest. Needs Nano.
- [ ] Q3 (2026-11-20, n until Phase 3): Who owns the `fusermount3` migration (ADR-0002 Tier T needs an owner and a milestone)? — interim: D15 proposes the milestone (replacement 1 with Phase 3); owner unassigned. Needs Nano.
- [ ] Q4 (2026-11-20, n): `fsk` licence, dependency list and release dates are unchecked; docs.rs was not reachable from the design session. — interim: `fsk` stays a design reference (D8). Any session with docs.rs or crates.io access can check.
- [ ] Q5 (2026-11-20, n): User-namespace FUSE mount (D15 replacement 2) rests on working knowledge (kernel 4.18+, distribution restrictions). — interim: confirm in the Phase 0 `/dev/fuse` spike before relying on it.
- [ ] Q6 (2026-11-20, n): Minimum macOS version for FSKit: `fskit-rs` says 15.4+, `fsk` says 26+. Also unknown: FSKit resource type for a filesystem with no block device; signing entitlement. — interim: macOS is low priority (D4); settle in a Phase 5 spike.
- [ ] Q7 (2026-11-20, n): Does this cloud workspace, or CI, have `/dev/fuse` plus root or `fusermount3`? Windows CI needs the ProjFS feature enabled on the runner. Not checked. — interim: Phase 0 checks both; until then conformance runs `memfs` with no mount on every OS.
- [ ] Q8 (2026-11-20, n): The architecture diagram in the Claude Doc is an embedded widget and was not exported. — interim: `../design.md` carries the layering in text and `SPEC.md` § Shape has an ASCII sketch; redraw the SVG into `docs/projects/rusty_fuse/` when wanted.
- [ ] Q9 (2026-11-20, n): `../design.md` is a condensed copy, not a byte-for-byte export; the Claude Doc rev 24 is the full text. — interim: treat the doc as the reference for wording; this repo copy is for cloud sessions.
