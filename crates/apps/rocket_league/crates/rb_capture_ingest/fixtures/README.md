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

## `human/`

Nine **real** excerpts of the owner's own BakkesMod capture sessions
(2026-09-04 to 09-10, keyboard and mouse, plugin 1.0 to 1.1), with the
input the game recorded on every tick. They come from the unmerged
`claude/rocket-league-server-clone-u74q45` branch of the archived
`baileyrd/rusty_bullet` (head `bcf8f69`), byte for byte. Each starts at
or just before a grounded, neutral frame so `rb-verify --self` seeds on
it. `rb_verify_cli/tests/human_captures.rs` bounds the port's error on
every one; the numbers below are main's on 2026-10-24.

| Fixture | Cut from (`raw/`) | Holds | Car mean / max (uu) | Ball mean / max (uu) |
|---|---|---|---|---|
| `throttle-jump` | `groundjumpthrottle03` 3.55–8.19 s | throttle from rest, a tapped jump, landing, a held jump | 1.94 / 5.55 | untouched |
| `boost-wall-entry` | `walldrive04` 26.85–29.10 s | at the 2300 cap, boost on, straight into the `+X` wall curve | 0.91 / 9.46 | untouched |
| `dodge-derailment` | session 1 (`test2`, not kept) 4.12–7.00 s | a ground jump and diagonal dodge | 6.96 / 24.14 | 18.55 / 83.85 |
| `airborne-hit` | `hittickjump01b` 5.60–9.90 s | boost, a jump 7 ticks before an aerial hit, the ball's flight to the goal | 1.54 / 8.61 | 1.50 / 8.53 |
| `hit-tick-jump` | `hittickjump01_v2` 43.81–47.90 s | a jump pressed 3 ticks before contact with all wheels down | 1.55 / 7.51 | 188.57 / 1023.82 |
| `wall-climb-crest` | `wall_curve02` 13.14–20.00 s | full-boost wall curve, climb to the ceiling fillet, crest, fall | 16.95 / 90.98 | 12.09 / 28.03 |
| `goal-shot-net-entry` | `goal_shot03` 8.82–13.40 s | a ground shot through the `+Y` goal mouth into the net | 1.52 / 2.00 | 6.01 / 20.91 |
| `clean-dodge` | `clean_dodge04` 6.28–8.60 s | a pure-left side dodge (`yaw = -1`) on open floor | 1.75 / 4.76 | untouched |
| `dodge-free-landing` | `onewheellanding06` 38.75–40.89 s | a hop and one-wheel landing with no dodge | 1.10 / 3.14 | untouched |

Two known capture defects, from the source branch: the analog axes are
recorded as zero throughout `hittickjump01` and `01b`, and `walldrive04`
has a dodge at 30.175 s with no `jump` press recorded. The files hold
poses and inputs only, no names or account ids.

`human/raw/` keeps the ten whole clips the excerpts were cut from; see its
README. No test reads them. The session-1 clip and `hit_tick_test` are not
there: the first was never vendored, the second is already at
`rusty_bullet/hit_tick_test.jsonl`.
