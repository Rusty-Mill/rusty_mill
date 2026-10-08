# ADR-0057: Scoring a scenario against its recording

- Status: Accepted
- Date: 2026-10-05
- Deciders: baileyrd
- Related: RB-VERIFY-003-FR-011, ADR-0056

## Context

A tape-bot capture is a whole BakkesMod session: frames before the state is
set, then the scenario. To score it we must find where the scenario starts
and line the port's prediction up tick by tick. The state may show up in the
capture a tick or two late, and the capture's input may belong to a tick or
the one before.

## Decision

- Find the start as the first recorded frame whose car is within 5 uu of the
  scenario's start location (a scenario without one cannot be aligned).
- Choose the lag, 0 to 3 ticks, that minimises position error over the first
  60 ticks.
- Compare the free-run prediction (not one-step): the point is the tick of
  first divergence from a known start and tape. Report mean and maximum
  position error and the first tick over 10 and 100 uu.
- Count a recorded input as playing the tape if it equals the tape's input
  for either neighbouring tick; report the rest as mismatches.

Alternatives considered: matching by recorded timestamp (the state-setting
tick is not marked in the file); one-step scoring (hides the divergence
growth the free run shows; the existing `--self-onestep` already covers a
capture's one-step error).

## Consequences

- A capture of any shipped scenario can be scored with one command.
- Start matching by position needs a start location in the scenario and a
  car that is not already there beforehand (the shipped scenarios put the
  car away from the pre-match position).
