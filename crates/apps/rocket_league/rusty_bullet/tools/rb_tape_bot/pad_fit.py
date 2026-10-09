"""Fits the boost pad pickup rule to pad logs (PARITY-PLAN workstream C step 2).

    python -I tools/rb_tape_bot/pad_fit.py replays/batch_<stamp> [more batches...]

Reads every `<name>_run<k>.jsonl` with its `<name>_run<k>.pads.jsonl` sidecar
(ADR-0071). The sidecar gives a pad's state only when it changes, so the state at
every capture row is rebuilt by matching each event's car position to a row.
For each hypothesis (which point of the car is tested, and the position of
which tick) a pad of kind `small`/`big` is taken when that point is within R
(planar) of the pad centre and within `H` above the pad, and the report gives
the interval of R every observation allows:

* a pickup at row f says `dist(f') <= R`;
* every row at which the pad was active and not taken says `dist > R`.

An empty interval (`lo >= hi`) means the hypothesis contradicts the data.
Rows are limited to a car near the floor for the radius fit (`GROUND_Z`); the
height fit uses the dropped-car tapes.
"""
import glob
import json
import math
import pathlib
import re
import sys

GROUND_Z = 40.0
FULL_TANK = 99.95
MAX_ROW_GAP_SECS = 1.5 / 120.0
STANDARD_PADS = 34  # Soccar's pad count; a first run can carry the previous match's list
# Octane hitbox centre in the car frame (the origin is not the box centre).
HITBOX_OFFSET = (13.88, 0.0, 20.75)

Vec = tuple[float, float, float]


def rotate(q: dict, v: Vec) -> Vec:
    """`v` rotated by the quaternion `q` (x, y, z, w)."""
    x, y, z, w = q["x"], q["y"], q["z"], q["w"]
    vx, vy, vz = v
    tx, ty, tz = 2 * (y * vz - z * vy), 2 * (z * vx - x * vz), 2 * (x * vy - y * vx)
    return (
        vx + w * tx + (y * tz - z * ty),
        vy + w * ty + (z * tx - x * tz),
        vz + w * tz + (x * ty - y * tx),
    )


class Run:
    """One capture with its pad sidecar: per-row car points and pad events."""

    def __init__(self, capture: pathlib.Path):
        self.name = capture.name
        frames = [json.loads(line) for line in capture.open(encoding="utf-8")]
        rows = [frame["cars"][0] for frame in frames]
        self.time: list[float] = [frame["timestamp_secs"] for frame in frames]
        self.origin: list[Vec] = []
        self.center: list[Vec] = []
        # A full tank does not consume a pad (padp_full_*), so those rows say nothing.
        self.boost: list[float] = [car["boost_amount"] for car in rows]
        for car in rows:
            p = car["position"]
            o = (p["x"], p["y"], p["z"])
            off = rotate(car["rotation"], HITBOX_OFFSET)
            self.origin.append(o)
            self.center.append((o[0] + off[0], o[1] + off[1], o[2] + off[2]))
        lines = [json.loads(line) for line in capture.with_suffix(".pads.jsonl").open(encoding="utf-8")]
        self.field = [l["field"] for l in lines if "field" in l][-1]
        # events: (row, [(pad, active)])
        self.events: list[tuple[int, list[tuple[int, bool]]]] = []
        anchor: tuple[int, int] | None = None  # (frame, row) of the first matched event
        for line in lines:
            if "pads" not in line:
                continue
            expected = None if anchor is None else anchor[1] + line["frame"] - anchor[0]
            row = self.match_row(line["car"], expected)
            if row is None:
                continue
            if anchor is None:
                anchor = (line["frame"], row)
            self.events.append((row, [(i, bool(a)) for i, a, _ in line["pads"]]))

    def match_row(self, car: list, expected: int | None) -> int | None:
        """The capture row whose origin equals the event's car position.

        A car standing still matches many rows; the one nearest `expected`
        (the first event's row plus the frames since) wins, else the first.
        """
        found = [
            i
            for i, o in enumerate(self.origin)
            if abs(o[0] - car[0]) < 0.02 and abs(o[1] - car[1]) < 0.02 and abs(o[2] - car[2]) < 0.02
        ]
        if not found:
            return None
        return found[0] if expected is None else min(found, key=lambda i: abs(i - expected))


def observations(run: Run, shift: float, points: list[Vec]):
    """Yields (kind, planar dist, dz, taken) for every pad and row of a run.

    `shift` picks the position tested for the pad state seen at row f: row f - shift,
    linearly interpolated between rows for a fraction (the pad test may run
    between the two ticks the capture shows).
    """
    if not run.events:
        return
    n = len(points)
    first = run.events[0][0]
    taken_rows: dict[int, set[int]] = {}
    state = {i: bool(True) for i in range(len(run.field))}
    # state at the first event: everything it lists.
    for i, a in run.events[0][1]:
        state[i] = a
    ev = 1
    for row in range(first, n):
        while ev < len(run.events) and run.events[ev][0] <= row:
            erow, changes = run.events[ev]
            for i, a in changes:
                if state[i] and not a and erow == row:
                    taken_rows.setdefault(row, set()).add(i)
                state[i] = a
            ev += 1
        whole, frac = divmod(shift, 1)
        src = row - int(whole) - (1 if frac else 0)
        if src < 0 or run.boost[row - 1] >= FULL_TANK:  # the tank before the step
            continue
        if frac and run.time[row] - run.time[row - 1] > MAX_ROW_GAP_SECS:
            continue  # a hole in the capture (RB-RESEARCH-O009): no interpolating across it
        px, py, pz = points[src]
        if frac:  # between row - whole - 1 (weight frac) and row - whole
            nx, ny, nz = points[src + 1]
            px, py, pz = (nx + (px - nx) * frac, ny + (py - ny) * frac, nz + (pz - nz) * frac)
        for i, pad in enumerate(run.field):
            dx, dy = px - pad["x"], py - pad["y"]
            if abs(dx) > 400 or abs(dy) > 400:
                continue
            was_active = state[i] or i in taken_rows.get(row, ())
            if not was_active:
                continue
            yield ("big" if pad["big"] else "small", math.hypot(dx, dy), pz - pad["z"], i in taken_rows.get(row, ()), pz)


def interval(runs: list[Run], shift: float, which: str, max_z: float = GROUND_Z):
    points_of = {"origin": lambda r: r.origin, "center": lambda r: r.center}[which]
    out = {}
    for kind in ("small", "big"):
        lo, hi, n_taken, n_pass = 0.0, 1e9, 0, 0
        for run in runs:
            for k, dist, _dz, taken, z in observations(run, shift, points_of(run)):
                if k != kind or z > max_z:
                    continue
                if taken:
                    lo = max(lo, dist)
                    n_taken += 1
                else:
                    hi = min(hi, dist)
                    n_pass += 1
        out[kind] = (lo, hi, n_taken, n_pass)
    return out


def height_interval(runs: list[Run], shift: float, inside: float = 150.0):
    """Interval of the height above the pad (origin z - pad z) that still picks it up.

    Only rows well inside the radius count (planar distance under `inside`), so
    the radius does not mix in: a pickup says `dz <= H`, a pass without one
    says `dz > H`.
    """
    out = {}
    for kind in ("small", "big"):
        lo, hi, n_taken = 0.0, 1e9, 0
        for run in runs:
            for k, dist, dz, taken, _z in observations(run, shift, run.origin):
                if k != kind or dist > inside:
                    continue
                if taken:
                    lo = max(lo, dz)
                    n_taken += 1
                else:
                    hi = min(hi, dz)
        out[kind] = (lo, hi, n_taken)
    return out


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    runs: list[Run] = []
    for batch in sys.argv[1:]:
        for sidecar in sorted(glob.glob(str(pathlib.Path(batch) / "*_run*.pads.jsonl"))):
            capture = pathlib.Path(sidecar.replace(".pads.jsonl", ".jsonl"))
            if capture.exists() and re.search(r"_run\d+\.pads\.jsonl$", sidecar):
                run = Run(capture)
                if len(run.field) == STANDARD_PADS:
                    runs.append(run)
                else:
                    print(f"skipped {run.name}: stale field list ({len(run.field)} pads)")
    print(f"{len(runs)} runs, {sum(len(r.events) for r in runs)} pad events")
    print(f"{'point':<8}{'tick':<12}{'kind':<7}{'R from':>9}{'R below':>10}{'width':>8}{'taken':>7}{'passes':>8}  verdict")
    for which in ("origin", "center"):
        for shift, label in ((0, "same row"), (0.75, "0.75 before"), (1, "row before")):
            res = interval(runs, shift, which)
            for kind in ("small", "big"):
                lo, hi, nt, npass = res[kind]
                ok = "consistent" if lo < hi else "CONTRADICTED"
                print(f"{which:<8}{label:<12}{kind:<7}{lo:>9.1f}{hi:>10.1f}{hi - lo:>8.1f}{nt:>7}{npass:>8}  {ok}")
    print("\nheight above the pad (origin z - pad z), rows within 150 uu planar:")
    for shift, label in ((0, "same row"), (0.75, "0.75 before"), (1, "row before")):
        for kind, (lo, hi, nt) in height_interval(runs, shift).items():
            print(f"  {label:<12}{kind:<7}H from {lo:6.1f}  H below {hi:7.1f}  ({nt} pickups)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
