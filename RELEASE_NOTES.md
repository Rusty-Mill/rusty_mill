# Release Notes

Tracks the **monorepo itself** — crate merges, workspace-wide CI, and
cross-crate changes (like the duplication sweeps below) — not each crate's
own internal changes, which are logged in that crate's own
`crates/<name>/RELEASE_NOTES.md` where one exists (many crates kept theirs
from before the merge; see ADR-0001 for why root and per-crate logs are
separate rather than one superseding the other).

One entry per merged PR against `main`, reverse chronological, each linking
to its PR. Bolded inline category tags (`**Added:**` / `**Changed:**` /
`**Fixed:**`), known limitations stated plainly.

---

## PR #65 — Deduplicate `rusty_rdp`'s byte cursor and split `rusty_ansder`'s two crates
**2026-09-01** · [#65](https://github.com/Rusty-Mill/rusty_mill/pull/65)

- **Fixed:** `rusty_rdp`'s hand-rolled byte Reader/Writer duplicated
  `rusty_wire`'s (a dependency `rusty_rdp` already declared but never
  used) — now re-exports `rusty_wire`'s cursor types.
- **Changed:** `rusty_ansder` bundled two unrelated libraries (an ASN.1 DER
  codec and a sovereign RAG/Q&A engine). Split the RAG engine into a new
  `rusty_rag` crate; `rusty_ansder` now holds just the DER codec.
- Also investigated and deferred (different-scoped tools sharing a name,
  not true duplication): `rusty_term` vs. `rusty_ansi`; `rusty_ansder`'s DER
  codec vs. `rusty_tls`'s hand-rolled DER; `rusty_http::Url` vs.
  `rusty_url::Url`.

## PR #10 — Collapse workspace duplication: to_wide, read_lines, SHA-1, IFS splitting, glob, raw-mode
**2026-08-27** · [#10](https://github.com/Rusty-Mill/rusty_mill/pull/10)

- **Fixed:** six of eight findings from a five-sweep duplication review
  (issues #1–#8) — `rusty_win32`'s 7x-duplicated `to_wide()` hoisted;
  `rsed`/`rawk`'s shared stdin-reading extracted to `read_lines()`;
  `rusty_git`/`rusty_term`'s independent SHA-1 implementations merged into
  a new `rusty_sha1` crate; `rush`'s two independent IFS-splitting
  implementations merged into `ifs_run_end()`; `rush`'s backtracking glob
  matcher now tries `rusty_regx::Glob` first; a duplicated Windows
  raw-mode flag transformation (`rusty_term`/`rusty_lines`) hoisted into
  `rusty_win32::console::raw_mode_core()`.
- **Known limitation:** two findings closed `no action` — Unix termios
  save/restore (`rusty_term`/`rusty_lines`) is the same shape by
  deliberate, different policy; a `no_std` rounding workaround in
  `rusty_font` (`round_nonneg` vs. `round_f32`) likewise.
- Filed #9 for the remaining gap (`rusty_regx::Glob` needs embedded `!(p)`
  negation support before rush's fallback matcher can be fully deleted) —
  a capability gap, not duplication; still open as of this writing.

## Earlier crate-import history

Every `Import <crate> into crates/<crate>` merge and the CI-scoping work
(`Speed up CI: affected-crate filtering, rust-cache, nextest, parallel
clippy`) predate this file. See `git log --oneline --merges` for the full
list — not backfilled entry-by-entry here since each import is already its
own reviewable commit with a descriptive message, and there are dozens of
them (see the crate table in `README.md` for the two-wave merge history).
