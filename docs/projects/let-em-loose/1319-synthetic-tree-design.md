# #1319 synthetic test tree, reference hardware and perf protocol

Card #1319 (let_em_loose, M2, child of #600). Status: **design for Nano's approval, 2026-11-28.**
Nothing is built until Nano approves it on the card (D25).

The frozen intent v0.1-stage1 (`dedup-manager` `INTENT.md`, D179) sets three targets "on reference
hardware": first full scan at most 30 min/TB, incremental rescan under 1 h after adding 50 GB, and
crash loss of at most a few minutes with resume on relaunch. Its `test_scale` is "a 500 GB synthetic
tree shaped like Nano's drive". This file says how the tree is made, where the targets are measured
and how. The frozen `perf` profile runs "on a 500 GB synthetic tree, scaled | Linux sandbox"
(600-builder-session-design.md §2, D66).

Decisions this rests on: D240 (generator location), D241 (protocol), D242 (image fixtures), D243
(what leaves Nano's machine), all `[default]`, and D246 (reference machine), `[stated]`: Nano chose
BaileyAi with a 1/10 slice on 2026-11-28 (section 4). No design question is open; the design itself
still waits for Nano's approval on the card.

## 1. Shape: measured from Nano's drives, aggregates only

`1319-drive-stats.ps1` (beside this file) walks the roots Nano names, read-only, and writes one JSON
file of aggregates. No names, paths or hashes appear in it (D243; #598 Q6).

| Output | Use in the generator |
|---|---|
| Files, directories, bytes per root, labelled `root1`, `root2`… with kind `local` or `network` | Local vs NAS split |
| Size histogram, log2 buckets, files and bytes per bucket | File size distribution |
| Depth histogram, entries-per-directory histogram (log2) | Tree shape |
| Top 25 extensions with at least 10 files each; the rest folded into `other` | File kind mix (images, video, other) |
| Zero-byte files, reparse points (symlinks, junctions), unreadable directories, paths over 259 chars | Edge-case counts |
| Files and bytes sharing an exact size with another file | Upper bound on duplicates |
| Only with `-SampleHash -MaxHashBytes <budget>` (off by default; the budget is required): whole same-size groups are chosen at random against first-pass metadata until the budget is used, the projected read is printed and reported, and the budget is enforced inside the read loop: each read request is charged in full before it is issued and never refunded (short, probe and failed reads included), and no file is read past its expected size plus one probe byte. A group that grew is skipped; a file that changed size, grows, ends early or fails is counted apart and never matched. Overlapping roots are refused, as typed and with symlinks and junctions resolved, so no file is counted twice. Output: files hashed, changed, incomplete and failed, bytes requested and returned, and path-based duplicate counts. Hashes stay in memory and are never written | Byte-identical duplicate rate. Path-based: hardlinks count as duplicates though they share storage, so it is not reclaimable space |

The same-pixels rate can't be measured without decoding images, so it stays a default (section 2)
unless Nano gives a figure. Until the stats arrive, the generator runs on the built-in default
profile below; the stats file replaces it, and its sha256 goes in the manifest.

Nano runs it in PowerShell 7.2 or later on the Windows desktop:

```powershell
pwsh -File .\1319-drive-stats.ps1 -Roots 'D:\','\\nas\share' -Out "$HOME\lel-drive-stats.json"
```

The report must be a new file in an existing folder outside every scanned root: an existing file or
a path inside a root is refused, and the file is created exclusively, so nothing on the roots is
written. "Outside" is checked as typed and with symlinks and junctions along the report folder and the
roots resolved, so a link into a root is refused. Bind mounts, subst drives, a mapped share of a local
folder and 8.3 short names are not resolved, so the report goes to an ordinary folder such as `$HOME`. Device paths (`\\?\`, `\\.\`) are refused for roots and the report. He reads the JSON before sending it; it is small enough to check by eye. `1319-drive-stats.test.ps1` tests the
budget, the in-loop cap, grown, short and failing reads, the hardlink case, the no-names rule and links into a root on a temporary fixture tree
only.

## 2. The generator: `lel_treegen`

A small rusty_mill crate, `crates/tools/lel_treegen` (D240). Builders never see rusty_mill (D18,
#1234). Only existing workspace dependencies: `sha2`, `serde_json`, `clap`. The PRNG is a hand-written
SplitMix64/xoshiro256** (about 30 lines) rather than `rand`'s `StdRng`, whose algorithm is not
promised to stay stable across versions; a seed must give the same bytes forever.

```
lel-treegen plan   --seed S --scale F [--profile stats.json] --manifest DIR   # pure: writes the manifest only
lel-treegen write  --manifest DIR --out TREE                                 # materialises it
lel-treegen delta  --manifest DIR --out TREE                                 # adds the seeded 50 GB x F delta
lel-treegen verify --manifest DIR --out TREE                                 # re-reads, checks every file
```

**Pure core, thin adapter.** `plan` is a pure function `(seed, scale, profile) -> Manifest` with no
I/O. File bytes are a pure function of `(seed, content_id)`, generated block by block at memory
speed, so `write` streams and never holds a file in memory. `write` and `verify` are the only code
touching the filesystem.

**What goes in the tree** (default profile; rates are shares of *units*, where a unit is an original
with its copies, a decoy or same-prefix pair, an image group or a single file, unless stated):

| Element | Default | Why it is there |
|---|---|---|
| Size distribution | log-normal-ish fitted to the stats histogram; default median 256 KiB, long tail to 8 GiB videos | Byte volume dominated by large files, count by small ones |
| Byte-identical duplicates | 15% of files have 1 to 4 copies; copies spread across folders and across the local/NAS split | Match kind 1, intent example "three identical 4 GB videos" |
| Same-pixels, different metadata | 3% of image files: groups of 2 or 3 sharing identical pixel data, differing in EXIF/text chunks; one member carries the richest metadata | Match kind 2, intent example `beach.jpg` / `copy (2).jpg` |
| Decoys: same name and size, different bytes | 1%: differ in one byte in the middle and one in the last block | Intent example "same name and size, different bytes"; defeats name+size or first-block shortcuts |
| Same first 64 KiB, different tail | 0.5% | Defeats prefix-hash-only grouping |
| NAS portion | 25% of bytes under `nas/` (exported as a share on the reference host) | "NAS shares as mounted paths", groups across both |
| Links leaving the roots | 50 symlinks pointing into `outside/`, which holds copies of in-tree files | `never: follow links outside the scanned roots`: following one creates false groups the manifest flags |
| Hardlinks | 20 pairs | One inode, two names: reclaiming one reclaims nothing; must not count as a reclaimable duplicate |
| Edge files | zero-byte files, an unreadable file and directory, Unicode and 255-byte names, a path over 260 chars, depth 40 | Robustness |

**Images (D242).** PNGs are written with stored (uncompressed) deflate, so identical pixels mean
identical IDAT bytes; metadata variants differ only in `tEXt`/`eXIf` chunks. JPEGs come from a
DC-only baseline encoder in the crate (each 8x8 block a flat value from the PRNG), about 150 lines:
valid files with distinct pixels per group and no codec dependency. Group variants differ only in
their APP1/EXIF segment. JPEGs are grayscale. The planner fixes each image's size in advance (an
upper bound on the encoded image plus 64 bytes per metadata step) and the writer pads to it exactly,
in a PNG ancillary chunk or a JPEG COM segment, so the manifest is complete before any byte is
written. Images are a small share of bytes; bulk volume comes from opaque files.

**Manifest (outside the tree, D240).** Written to a harness-only directory, never under the tree root
or any sandbox or builder mount. JSON Lines, one record per file (path, size, content id, kind,
group id, link target), then one per expected group (match kind, members, suggested keeper for
same-pixels groups). `manifest.sha256` is the sha256 of the canonical manifest; with the seed, scale
and profile digest it names the tree exactly, and it is what gets recorded on #600. `verify` proves a
materialised tree matches it.

**Delta.** `delta` adds 50 GB × scale of new files from the same seed stream: new uniques, new copies
of existing files (so groups grow), and new same-pixels variants. Additions only, as the intent
says.

## 3. Space and time

| Scale | Tree | Delta | Free space needed | Write time at 500 MB/s |
|---|---|---|---|---|
| 0.1 | 50 GB | 5 GB | ~60 GB | ~2 min |
| 1.0 | 500 GB | 50 GB | ~560 GB | ~19 min |

## 4. Reference hardware: decided (D246)

The frozen `perf` profile runs in the Linux sandbox on BaileyAi, "scaled" (D66, D53). Nothing named
the reference machine, the scale factor, or where the 30 min/TB verdict is judged, so the expert
escalated it. Nano chose (a) on 2026-11-28 (D246):

- **a) Chosen.** BaileyAi and its data disk are the reference. `perf` runs a 1/10 slice from the
  same seed (50 GB tree, 5 GB delta), scaled linearly, calibrated by one full 500 GB run on BaileyAi
  that checks linear scaling holds. Windows desktop timings from the shadow run (#1321) are reported
  for information only. Keeps the approved profile set and host decision.
- Not chosen: b) the verdict judged on the full tree on the Windows desktop and NAS (would have
  changed D66 §2); c) the full 500 GB tree for every `perf` run, no scaling (about 10x the run time).

The card records the disk model, filesystem, free space, CPU and RAM of BaileyAi; the
generation prompt (section 6) collects them. The NAS portion is served from a second mount on the
same host.

## 5. Measurement protocol (D241)

- **Warm:** a repeat run after one discarded run. **Cold:** the harness drops the page cache on the
  host (needs root, so not inside the sandbox) before each run.
- Five runs each; report the median and the spread.
- **First scan:** wall time from launch to "scan complete", on a fresh index. Target scaled: 500 GB
  × 30 min/TB = 15 min at full scale, 1.5 min on the 1/10 slice.
- **Incremental:** after a complete scan, apply `delta`, then rescan; target under 1 h at full
  scale, scaled by the same factor.
- **Crash loss:** SIGKILL (`taskkill /F` on Windows) at 10%, 50% and 90% of files scanned, judged
  as wall time from relaunch until progress regains the pre-kill point; re-hashed bytes reported
  beside it. Target: a few minutes.
- **Correctness check on every run:** the groups the candidate reports are compared with the manifest
  by the harness, which alone reads it.

## 6. Slices after approval

One PR each, stacked, each building and testing alone; review requested from dottiusmaximus.

1. **Core:** crate, PRNG, profile parsing, `plan`, manifest and digest. Pure; tests: same seed gives
   the same manifest digest, rates within tolerance, scale 0.1 is a prefix-consistent slice, decoys
   never share a group.
2. **Writer:** `write`, `delta`, `verify`, PNG and DC-only JPEG writers. Tests on a tiny scale in a
   temp dir: files decode, same-pixels groups share pixel bytes, `verify` catches a flipped byte.
3. **Generation:** a paste-ready Claude Code prompt in project files for BaileyAi that builds the
   tool, generates the tree and records hardware facts and `manifest.sha256`; the digest goes on #600
   and the protocol stays in this file.

## 7. For Nano

1. Approve this design, or say what changes.
2. ~~Pick the reference machine (section 4).~~ Done: (a), BaileyAi with a 1/10 slice, 2026-11-28 (D246).
3. Optional, any time: run `1319-drive-stats.ps1` and send the JSON. Without it the default profile
   is used and the card says so.
