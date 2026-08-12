# Release Notes

User-facing and contributor-facing changes to this repo, one entry per merged
PR against `main`, newest first. Started partway through the project's life
(see `git log` for everything before this file existed) rather than
backfilling the full history.

---

## PR #47 — Add standard governance files (repo-config)
**2026-08-12** · [#47](https://github.com/baileyrd/rusty_serde/pull/47)

- **Added:** the standard repo-config governance-file set - `CONTRIBUTING.md`,
  `CODE_OF_CONDUCT.md`, `SECURITY.md`, `CHANGELOG.md`, this file,
  `ARCHITECTURE.md` (boundary table and non-goals filled in for real, not
  left as scaffold), and an ADR seed at `docs/adr/0001-template.md`.
- **Fixed:** a stale README claim ("the JSON parser always allocates
  Strings") - `&'de str`/`Cow<'de, str>` fields already borrow zero-copy
  when there's no escape sequence to resolve, per already-passing tests.
  What's actually missing is the `#[serde(borrow)]` attribute itself, for
  a different reason than upstream (see `ARCHITECTURE.md`'s non-goals).
- **Known limitation:** the security contact (`@baileyrd`) is a
  repo-config default, not a confirmed real one; CI isn't yet wired up as
  a required status check in branch protection.
