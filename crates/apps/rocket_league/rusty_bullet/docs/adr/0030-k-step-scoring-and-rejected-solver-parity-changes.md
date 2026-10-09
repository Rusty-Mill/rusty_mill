# ADR-0030: Score k-step predictions; keep the solver's row order and correction factors

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-VERIFY-003-FR-008, FR-006; RB-PHYSICS-001-FR-110, FR-034;
  ADR-0029
- Supersedes/Superseded by: —

## Context

One-step error (`--self-onestep`) is down to a `test2` car mean of 1.59
uu/s, and it barely sees position effects such as contact correction. A
free run (`--self`) is chaotic after `test2`'s kickoff: three candidate
solver changes moved its mean car position between 668 and 1,624 uu. So
it cannot rank them.

Three solver differences from RocketSim remained:
- Bullet resolves all normal rows, then all friction rows, in each
  iteration, and skips a friction row while its normal impulse is zero.
  The port interleaves the two per contact.
- RocketSim sets `m_erp2 = 0.8`; the port uses 0.2.
- Bullet applies split-impulse turn velocity times
  `m_splitImpulseTurnErp` (0.1); the port applies all of it.

## Decision

- `rb-verify --self-kstep <capture> [k]` (`k_step_score`,
  `world::simulate_recorded_k_step`) predicts every frame `k` ticks ahead
  (default 30, a quarter second) from the recorded frame `k` before it,
  and scores it like `--self`.
  - Each prediction starts from a clone of the one-step world, so drive
    state carries on; `PhysicsWorld` is `Clone`.
  - `k = 1` reproduces one-step.
- None of the three solver changes is adopted. At k = 30 they moved no
  capture's error by more than about 1%, and ERP2 0.8 made the ball worse:

  | Variant | `test2` ball / car position (uu) / car velocity (uu/s) |
  |---|---|
  | baseline | 0.56 / 3.29 / 26.27 |
  | Bullet row order (with or without the skip) | 0.56 / 3.29 / 26.25–26.27 |
  | ERP2 0.8 | 0.61 / 3.30 / 26.43 |
  | turn ERP 0.1 | 0.56 / 3.29 / 26.19 |
  | ERP2 0.8 + turn ERP 0.1 | 0.61 / 3.28 / 26.12 |

  `front` and `side` stayed within 0.03 uu/s.

## Consequences

- The k = 30 baseline (ball / car position / car velocity):

  | Capture | Ball (uu) | Car position (uu) | Car velocity (uu/s) |
  |---|---|---|---|
  | `test2` | 0.56 | 3.29 | 26.27 |
  | `front` | 0 | 0.11 | 0.84 |
  | `side` | 0 | 0.10 | 0.52 |

  Later changes report against it.
- The three solver differences stay documented, not ported. Revisit them
  if a capture that exercises them (stacked or resting contacts) shows
  error.
