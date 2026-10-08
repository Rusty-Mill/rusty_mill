# Fixtures

## `example.capture.jsonl`

A **synthetic, hand-authored** capture file (5 ticks: a car driving toward
the ball, boosting, then jumping and pitching into a dodge) in the JSON
Lines format ADR-0005 decided. Used only to unit-test
`rb_capture_ingest`'s parser against every field the format defines
(including a car's `input`, which is always present in a real capture,
unlike `rb_replay_ingest`'s frames).

Timestamps start at `11.78` (not `0.0`) so this fixture actually overlaps
`rb_replay_ingest`'s vendored replay fixture's real timeline —
`rb_verify_cli`'s tests and manual verification runs score the two
against each other, and `rb_domain::divergence::score`'s timestamp-
tolerant alignment (`RB-VERIFY-003-FR-003`) can only produce a non-empty
result if their timestamps genuinely overlap. `11.78`s is roughly when
the vendored replay's ball actually spawns (frames before that are
omitted — see `rb_replay_ingest/fixtures/README.md`); an earlier version
of this fixture started at `0.0` and silently never overlapped it at all,
undetected until real timestamp alignment landed (see `RB-VERIFY-003`'s
change history).

- **Not a real BakkesMod capture.** No such file exists yet: the
  BakkesMod-side plugin that would write one (`RB-VERIFY-002-FR-001`) has
  not been built — this sandboxed environment has no Rocket League,
  BakkesMod, or Windows to build/run it on (the same practical blocker
  documented for `RB-RESEARCH-O002`). Unlike `rb_replay_ingest`'s vendored
  third-party fixture (a real replay, just not the owner's own), there is
  no real capture file of any provenance to vendor here at all.
- **What it does *not* satisfy**: `RB-VERIFY-002`'s acceptance criteria
  call for a capture recorded from a real local/offline match, cross-checked
  against BakkesMod's own debug overlay at a manually-verified timestamp.
  This fixture only proves `rb_capture_ingest`'s JSON-Lines parsing logic is
  correct against the format's own schema — it says nothing about whether
  that schema matches what a real BakkesMod plugin would actually produce,
  since none has been written yet.
