# `aisf_triage_labels.jsonl` — labeling notes

40 hand-labeled scenarios for `env: { name: aisf_triage }`
(`crates/skillopt-envs/src/aisf_triage.rs`), replacing the original 8
illustrative placeholder rows. Format: one JSON object per line —
`id`, `split` (`train`/`val`/`test`), `scenario` (AISF's `TriageScenario`
shape, `{"issues": [...]}`, passed through verbatim), `expected_priority`
(`"P0"`-`"P3"`).

## Priority rubric

AISF's triage prompt doesn't define P0-P3 itself (see
`prompts/triage.md` in the AISF repo), so labels here need a stated
standard to be principled rather than vibes-based — and so a future
disagreement is traceable to a rule, not re-litigated from scratch:

- **P0 — Critical.** Full outage or an effectively unusable core flow
  (login, checkout, primary API) for all or most users; an actively
  exploitable security hole; irreversible data loss; no workaround.
- **P1 — High.** Major functionality broken or badly degraded for a real
  but bounded subset of users (a platform, a plan tier, a browser), with
  no reasonable workaround — or a serious-but-not-yet-exploited security
  weakness, or silent/partial data integrity loss (a truncated export, a
  double charge). Short of a full outage.
- **P2 — Medium.** A genuine bug with a viable workaround, or narrow
  blast radius, or moderate UX/performance friction. The largest,
  least-dramatic bucket in a real tracker.
- **P3 — Low.** Cosmetic/copy issues, low-urgency enhancement requests,
  non-urgent docs fixes, edge-case-of-an-edge-case bugs with negligible
  impact.

## Multi-issue scenarios

AISF's triage prompt says "fetch open issues, classify the
highest-priority one" — plural in, one priority out. Most scenarios here
are single-issue (the common case, and the cleanest signal), but four
(`train-22`, `train-23`, `train-24`, `val-08`, `test-08`) list 2+ issues
in one `scenario`, so `expected_priority` is the priority of the worst
issue in that batch, not "the" issue — this is the only way to exercise
whether the agent is actually picking the highest-priority item rather
than just classifying whatever it's shown.

## Split sizes and known limitation

24 train / 8 val / 8 test. Every split includes all four priority
levels, but at 8 examples, val/test scores can swing by 0.125 per
example — the same small-split noise `docs/USAGE.md` §5 already
documents for `synthetic_arithmetic`'s smaller configs. Real training
runs against this file should expect that noise rather than read a
single val/test score as definitive; growing the set (more hand-labeled
rows, same format) is the fix if it turns out to matter.

Category coverage across the 40: outages/availability, security
(auth bypass, XSS, IDOR/tenant isolation, privilege escalation), payments/
billing, data loss/integrity, auth, mobile, browser-specific, performance,
notifications, search, UI/UX, docs, and low-priority enhancement requests
— chosen to span the P0-P3 boundary with real-sounding titles rather than
titles that announce their own priority.
