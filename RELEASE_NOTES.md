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

## Fourth-wave merge — `rusty_test`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_test` — the `portable-runtime-contract` spike:
  one execution contract (`contract`), a per-host adapter (`compat`), a
  verification layer (`conformance`), and three reference tools
  (`stat-tool`, `proc-runner`, `pty-shell`). Merged via `git subtree` with
  full history; its nested `[workspace]` table removed and its six crates
  added to this root's `members`.
- **Changed:** its `[workspace.package]` didn't collide with this root's
  (same edition, same license), so its crates keep inheriting via
  `field.workspace = true` — the `rusty_search` treatment, not
  `rusty_db`'s. Only `publish = false` was new here. `thiserror` was
  deliberately left un-hoisted: `rusty_test` wanted `"2.0"`, this root
  pins `"1"` for `rusty_db`/`rustils`, so `contract` keeps a literal
  `thiserror = "2"` instead of forcing a major bump on unrelated crates.
- **Fixed:** `conformance`'s `tests/layering.rs` reads the workspace
  manifest to enforce the layer model, resolving it two directories above
  its own crate and requiring a declared layer for every member found.
  Post-merge that is this monorepo's root — four levels up, ~100 members —
  so three of its four tests panicked. Repointed and filtered through a
  `GROUP_PREFIX` constant; the check's logic is otherwise untouched and all
  four tests pass, alongside the group's other 27.
- No dependency swaps: nothing in `rusty_test` depended on a sibling in
  this workspace.

## Fourth-wave merge — `rusty_croc`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_croc` — a Rust port of
  [croc](https://github.com/schollz/croc), wire-compatible with stock croc
  v10 (PAKE code phrases, relay, local-network hand-off, resume). Merged
  via `git subtree` with its full commit history, as with every prior
  crate import.
- **Fixed:** four `GenericArray::from_slice` calls in `crypt.rs` (AES-256-GCM
  and XChaCha20-Poly1305 nonces). The standalone repo's lockfile pinned
  `generic-array` 0.14.7; this workspace resolves 0.14.9, which deprecates
  the crate wholesale, so `-D warnings` turned them into errors. Rewritten
  to the `From<&[T]> for &GenericArray` conversion `from_slice` delegates
  to — no behavior change, 49 tests pass unmodified.
- No dependency swaps: `rusty_croc` depends only on crates.io crates, not
  on any sibling in this workspace. Its nightly-only `fuzz/` harness keeps
  its own `[workspace]` table and is excluded from this one, same as
  `rusty_tls/fuzz` and `rusty_lsp/fuzz`.

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
