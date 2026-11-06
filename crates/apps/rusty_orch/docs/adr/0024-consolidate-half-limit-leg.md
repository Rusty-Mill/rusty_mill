# ADR-0024: `rusty_orch consolidate`, D44's half-limit leg

- **Status:** Accepted (Let Em Loose D114, D115, build defaults D118 and D120)
- **Date:** 2026-11-06

## Context

ADR-0023 built D44's count leg. The other leg schedules a cleanup when
cumulative erosion reaches half the limit. D114 measures it as the repo
head against the last cleanup's base. D115 makes the limits Nano's
approved pair, recorded by the host with `bbp erosion approve`.

## Decision

- orch reads the limits with `bbp erosion limits --dir D`, in plan and
  `--execute` modes, so `--bbp-bin` is no longer `--execute`-only. orch
  never parses the host's record file.
- With no record the leg is `inactive`, and a waiting run measures
  nothing, as before.
- With a record, every run that isn't busy measures HEAD's shares. If the
  journal has no `last_base` yet, `--execute` stores HEAD as the start and
  does not fire.
- The leg fires when either share rose and `2 * rise >= limit` (D120), so
  a 0 bp limit never fires on an unchanged tree.
- `last_base` moves on the same events as the count marker: an open that
  settles (with the levels measured for it, kept in `pending.base`) and a
  zero-floor skip. Either leg moves both.
- The report adds `trigger` (`count` or `half`), `half` (`inactive`,
  `start`, `below` or `reached`), `limits` and `last_base`.

## Consequences

- A run with limits recorded costs one `erosion check --report-only` even
  when no cleanup is due.
- A cleanup that never merges leaves `last_base` at the levels it was
  opened against, so the leg fires again only on new erosion.
