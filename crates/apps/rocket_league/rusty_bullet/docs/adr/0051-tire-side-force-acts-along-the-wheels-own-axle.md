# ADR-0051: A tire's side force acts along the wheel's own axle

- Status: Accepted
- Date: 2026-10-04
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-132, FR-080, FR-090
- Supersedes/Superseded by: —

## Context

`test2.jsonl` 5.77-5.92 s: after the kickoff hit and jump, a tilted car has
one wheel down at origin height 30-47 uu. The recording's sideways
acceleration there (about 5000 uu/s^2 along the car's right axis) comes with
an up acceleration of 1100-1260 against the jump's 1458. The sim's per-tick
error was 3-6 uu/s too much up and 3.5 too little sideways. The sim flattened
the wheel axle onto the contact surface, so its side force had no vertical
part at all.

## Decision

- `ground::wheel_impulses` uses `right * cos(angle) - forward * sin(angle)`
  as the axle, unprojected. The rolling direction keeps its projection.

Alternatives considered:

- Project only with several wheels down. Rejected: no recording needs it,
  and it adds a rule for nothing.

## Consequences

- `test2` 5.77-5.92 s mean one-step velocity error 4.87 -> 0.44 uu/s;
  k = 30 car 4.73 -> 2.55. `hitjump` k = 30 car 11.23 -> 10.82, `side`
  0.51 -> 0.45, `front` unchanged.
- The 18.358 s bottomed-out landing tick goes 6.2 -> 7.3 uu/s while its
  neighbours improve (15-21 -> 7-11); its test bound moves 5 -> 8. The jump
  from rest test bound moves 1.0 -> 1.5 uu/s horizontal (the capture's own
  frames are unchanged).
- On a level car on a flat floor nothing changes: the axle is already
  horizontal.
