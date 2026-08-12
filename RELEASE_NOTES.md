# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

No version tags yet (pre-1.0) — tracked one entry per PR against `main`, reverse
chronological.

---

## PR #3 — Add standard governance scaffolding and CI
**2026-08-12** · [#3](https://github.com/baileyrd/rusty_time/pull/3)

- **Added:** standard governance file set (README, CONTRIBUTING, CODE_OF_CONDUCT,
  SECURITY, CHANGELOG, this file, ARCHITECTURE, ADR seed, PR/issue templates).
- **Added:** `.github/workflows/ci-rust.yml` — `cargo fmt --check`, `cargo clippy
  --all-targets -- -D warnings`, and `cargo test`, run as a `check` job. It checks
  out `rusty_std` as a sibling directory to satisfy the `path = "../rusty_std"`
  dependency (see ARCHITECTURE.md).
- **Fixed:** ran `cargo fmt` across `src/lib.rs` so the new fmt-check gate starts
  green rather than red on day one.
- **Known limitation:** branch protection (required status check, merge-commit-only)
  isn't set from code — it's a manual one-time GitHub settings change, documented in
  CONTRIBUTING.md's review & merge section.

## PR #2 — Add RFC 3339 parsing and a working `DateTime::timestamp()`
**2026-08-12** · [#2](https://github.com/baileyrd/rusty_time/pull/2)

- **Added:** `DateTime::parse` (RFC 3339, with a matching `FromStr` impl) — handles
  `Z`/lowercase designators, fractional seconds, and numeric `±HH:MM` offsets,
  rejecting malformed input rather than panicking.
- **Fixed:** `DateTime::timestamp()`, which previously always returned `0`, now
  computes the real Unix timestamp via a proleptic-Gregorian civil-calendar day
  count (Howard Hinnant's `days_from_civil`).
- **Fixed:** `Date::from_ymd` now validates the day against the actual days in the
  given month/year (leap years included) instead of accepting any day 1-31
  regardless of month.
- 9 new unit tests (11 total, all passing); `cargo clippy` clean.
- **Known limitation:** no IANA timezone-name support, only fixed numeric offsets —
  see ARCHITECTURE.md non-goals.
