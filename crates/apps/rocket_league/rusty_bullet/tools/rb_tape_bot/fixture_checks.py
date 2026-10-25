"""Read physics constants straight off the committed game captures (PHYSICS-PARITY pass 2).

    python -I tools/rb_tape_bot/fixture_checks.py [all|caps|ball-spin|boost-taps|ball-roll|ball-rest|bounces|pads]

Reads `fixtures/*.jsonl` (BakkesMod state and recorded inputs, 120 Hz) with the standard library
only and prints the numbers `docs/research/PHYSICS-PARITY-VS-ROCKETSIM.md` quotes. Nothing here
runs the port; the captures are the game's own, so each result is a measurement of the game.
Tests: `python -I -m unittest discover -s tools/rb_tape_bot -p "test_fixture_checks.py"`.
"""
import json
import math
import pathlib
import statistics
import sys

FIXTURES = pathlib.Path(__file__).resolve().parent / "fixtures"
DT = 1 / 120
BALL_DRAG = 0.03  # v *= (1 - drag) ** dt, as in the port and RocketSim; removed before reading friction
# The 28 small boost pad centres (x, y), mirrored; RocketSim RLConst.h LOCS_SMALL_SOCCAR.
SMALL_PADS = [
    (0, -4240), (-1792, -4184), (1792, -4184), (-940, -3308), (940, -3308), (0, -2816),
    (-3584, -2484), (3584, -2484), (-1788, -2300), (1788, -2300), (-2048, -1036), (0, -1024),
    (2048, -1036), (-1024, 0), (1024, 0), (-2048, 1036), (0, 1024), (2048, 1036),
    (-1788, 2300), (1788, 2300), (-3584, 2484), (3584, 2484), (0, 2816), (-940, 3308),
    (940, 3308), (-1792, 4184), (1792, 4184), (0, 4240),
]
RS_CYL_RAD_SMALL = 144  # RocketSim RLConst.h CYL_RAD_SMALL, first acquisition (BoostPad.cpp)
PORT_SMALL_PAD_RADIUS = 176  # rb_physics_bullet pads.rs, fitted to the game (ADR-0072)


def load(path):
    """Read a capture. Only a crash-truncated final record may be skipped (ADR-0005); a malformed
    record anywhere else would silently change counts and join separate observations, so it raises
    with the file name and line number."""
    lines = [(n, text) for n, text in enumerate(path.read_text().splitlines(), 1) if text.strip()]
    frames = []
    for i, (n, text) in enumerate(lines):
        try:
            frames.append(json.loads(text))
        except json.JSONDecodeError as err:
            if i == len(lines) - 1:
                continue
            raise ValueError(f"{path.name}:{n}: malformed record ({err.msg})") from err
    return frames


def captures():
    return [(p.name, load(p)) for p in sorted(FIXTURES.glob("*.jsonl"))]


def norm(v):
    return math.sqrt(v["x"] ** 2 + v["y"] ** 2 + v["z"] ** 2)


def plateaus(values, cap, tol=1e-3, settle=20):
    """Runs of samples at `cap` (within `tol`): (first index, length, value just before, dynamic).

    A run is dynamic when it starts after the first `settle` samples (so it is not a seeded start
    state) and was entered from at least half the cap but below it (so it was reached by
    accumulating, not by a one-step jump from rest, which a collision or a state-set can also do).
    """
    out, i = [], 0
    while i < len(values):
        if values[i] < cap - tol:
            i += 1
            continue
        j = i
        while j < len(values) and values[j] >= cap - tol:
            j += 1
        before = values[i - 1] if i else None
        out.append((i, j - i, before, i > settle and before is not None and cap / 2 <= before < cap - tol))
        i = j
    return out


def report_plateaus(label, data, cap, getter):
    """Print how many captures reach `cap` dynamically and with what lengths; list the others by name."""
    dynamic, other = [], []
    for name, frames in data:
        for start, length, before, dyn in plateaus([getter(f) for f in frames], cap):
            (dynamic if dyn else other).append((name, start, length, before))
    print(f"{label} at {cap:g}: {len(dynamic)} dynamic plateaus in {len({d[0] for d in dynamic})} captures "
          f"({sum(d[2] for d in dynamic)} frames); {len(other)} seeded or one-step runs ({sum(o[2] for o in other)} frames)")
    for name, start, length, before in dynamic[:4]:
        print(f"  dynamic: {name} from frame {start} (file line {start + 1}), {length} frames, previous {before:.5g}")


def caps(data):
    """Largest values seen, and whether the caps are reached dynamically or only in seeded starts."""
    best = {"ball spin": (0, ""), "ball speed": (0, ""), "car speed": (0, ""), "car spin": (0, "")}
    for name, frames in data:
        for f in frames:
            for key, v in (("ball spin", f["ball"]["angular_velocity"]), ("ball speed", f["ball"]["velocity"])):
                best[key] = max(best[key], (norm(v), name))
            for c in f["cars"]:
                best["car speed"] = max(best["car speed"], (norm(c["velocity"]), name))
                best["car spin"] = max(best["car spin"], (norm(c["angular_velocity"]), name))
    for key, (value, name) in best.items():
        print(f"max {key}: {value:.5g} ({name})")
    report_plateaus("car speed", data, 2300.0, lambda f: norm(f["cars"][0]["velocity"]))
    report_plateaus("car spin", data, 5.5, lambda f: norm(f["cars"][0]["angular_velocity"]))


def ball_spin(data):
    """Frames at the ball's angular velocity cap, whether any exceed it, and which are dynamic."""
    above = sum(1 for _, fr in data for f in fr if norm(f["ball"]["angular_velocity"]) > 6.0001)
    print(f"ball spin above 6.0001 rad/s: {above} frames")
    report_plateaus("ball spin", data, 6.0, lambda f: norm(f["ball"]["angular_velocity"]))
    for name, frames in data:
        if name == "probe_ball_roll.jsonl":
            start = plateaus([norm(f["ball"]["angular_velocity"]) for f in frames], 6.0)[0][0]
            print(f"  probe_ball_roll: spin reaches 6.0 at frame {start} (file line {start + 1}) with ball speed "
                  f"{norm(frames[start]['ball']['velocity']):.1f} uu/s, well before the rolling transition (see ball-roll)")


def activations(series, max_hold=14, tail=16):
    """Boost button runs of at most `max_hold` frames and what the tank did afterwards.

    `series` is a list of (timestamp, boost_pressed, boost_amount). Each activation's window ends
    at the next activation's first frame (or `tail` frames after the release, whichever is
    sooner), so a second press cannot be counted as part of the first burn. A drop is a frame whose
    tank is lower than the previous frame's; ticks are counted from timestamps, not frame offsets,
    so a missing sample shows up as a gap rather than a shorter burn.
    """
    runs, i = [], 0
    while i < len(series):
        if not series[i][1]:
            i += 1
            continue
        j = i
        while j < len(series) and series[j][1]:
            j += 1
        runs.append((i, j))
        i = j
    out = []
    for n, (i, j) in enumerate(runs):
        if j - i > max_hold:
            continue
        end = min(j + tail, runs[n + 1][0] if n + 1 < len(runs) else len(series), len(series))
        drops = [k for k in range(i, end - 1) if series[k + 1][2] < series[k][2] - 1e-6]
        info = {"held": j - i, "start_amount": series[i][2], "drops": len(drops), "span_ticks": 0, "gaps": 0}
        if drops:
            first, last = drops[0] + 1, drops[-1] + 1
            info["first_offset"], info["last_offset"] = first - i, last - i
            info["span_ticks"] = round((series[last][0] - series[first][0]) / DT) + 1
            info["gaps"] = sum(1 for k in range(first, last) if series[k + 1][0] - series[k][0] > 1.5 * DT)
            info["end_amount"] = series[last][2]
        out.append(info)
    return out


def boost_taps(data):
    """Short boost button holds and the ticks on which the tank actually drops afterwards."""
    for name, frames in data:
        series = [(f["timestamp_secs"], f["cars"][0]["input"]["boost"], f["cars"][0]["boost_amount"])
                  for f in frames if f["cars"]]
        for a in activations(series):
            if not a["drops"]:
                print(f"{name}: held {a['held']} frames; no drain (tank {a['start_amount']:.2f})")
                continue
            print(f"{name}: held {a['held']} frames; {a['drops']} drops at offsets +{a['first_offset']}..+{a['last_offset']}, "
                  f"spanning {a['span_ticks']} ticks ({a['gaps']} sample gaps), {a['start_amount']:.2f} -> {a['end_amount']:.2f}")


def friction_samples(rows, drag=BALL_DRAG):
    """Sliding friction deceleration per sample, with the port's exponential drag removed.

    Drag is v *= (1 - drag) ** dt, so dv/dt = ln(1 - drag) * v - friction while sliding and
    friction = ln(1 - drag) * v - dv/dt. `rows` are (time, speed, spin, height).
    """
    k = math.log(1 - drag)
    return [k * rows[i][1] - (rows[i + 1][1] - rows[i - 1][1]) / (rows[i + 1][0] - rows[i - 1][0])
            for i in range(1, len(rows) - 1) if rows[i][1] > 600 and abs(rows[i][3] - 93.2) < 0.2]


def ball_roll(data):
    """Sliding friction, spin growth and the speed at which sliding ends (probe_ball_roll)."""
    frames = dict(data)["probe_ball_roll.jsonl"]
    rows = [(f["timestamp_secs"], f["ball"]["velocity"]["x"], norm(f["ball"]["angular_velocity"]), f["ball"]["position"]["z"])
            for f in frames]
    est = friction_samples(rows)
    print(f"sliding friction (exponential drag removed): {statistics.mean(est):.1f} uu/s2, sd {statistics.pstdev(est):.1f}, n={len(est)}")
    t0 = next(i for i, r in enumerate(rows) if r[0] >= 0.3)
    t1 = next(i for i, r in enumerate(rows) if r[0] >= 0.8)
    print(f"spin growth: {(rows[t1][2] - rows[t0][2]) / (rows[t1][0] - rows[t0][0]):.3f} rad/s2")
    capped = next(i for i, r in enumerate(rows) if r[2] > 5.9999)
    rolling = next(i for i in range(capped, len(rows)) if rows[i][2] < 5.999)  # spin starts tracking v / radius
    print(f"rolling begins at frame {rolling} (file line {rolling + 1}): v = {rows[rolling][1]:.1f} uu/s, spin {rows[rolling][2]:.3f}, "
          f"v / spin = {rows[rolling][1] / rows[rolling][2]:.1f} (6 x 91.25 = {6 * 91.25})")
    print(f"friction window: frames with speed above 600 uu/s and height within 0.2 of 93.2 (the {len(est)} frames), "
          "central difference, exponential drag removed")


def freeze(rows):
    """The first frame at which a moving ball is held at exactly zero speed, or None.

    `rows` are (time, speed, spin) per frame, the first the state set. Returns (frame, seconds since
    the first frame, the most speed and spin on any stepped frame before it): the ball stayed under
    the thresholds on all of those, so the thresholds lie above them."""
    for k in range(2, len(rows)):
        if rows[k][1] == 0 and rows[k][2] == 0 and rows[k - 1][1] > 0:
            moving = rows[1:k]
            return k, rows[k][0] - rows[0][0], max(r[1] for r in moving), max(r[2] for r in moving)
    return None


def awake_bound(rows, dwell):
    """What a ball that is never frozen says about the thresholds, or None if it is shorter than `dwell`.

    Over every window of `dwell` seconds of stepped frames, the ball was at or above the linear
    threshold or the angular one on some frame (or it would have been frozen). Returns the window
    whose peak speed is lowest, as (its peak speed, its peak spin): either the linear threshold is at
    most that speed or the angular one is at most that spin. Its slowest frame alone bounds nothing."""
    stepped = rows[1:]
    best = None
    for i, first in enumerate(stepped):
        j = next((j for j in range(i, len(stepped)) if stepped[j][0] - first[0] >= dwell), None)
        if j is None:
            break
        window = stepped[i:j + 1]
        peak = (max(r[1] for r in window), max(r[2] for r in window))
        if best is None or peak[0] < best[0]:
            best = peak
    return best


def ball_rest(data):
    """When a ball rolling without sliding is frozen, and what that bounds (probe_ball_rest_*, board b19)."""
    frozen, awake = [], []
    for name, frames in data:
        if not name.startswith("probe_ball_rest_"):
            continue
        rows = [(f["timestamp_secs"], norm(f["ball"]["velocity"]), norm(f["ball"]["angular_velocity"])) for f in frames]
        hit = freeze(rows)
        if hit is None:
            awake.append((name, rows))
            continue
        k, secs, speed, spin = hit
        frozen.append((speed, spin, secs))
        print(f"{name}: frozen at frame {k} (file line {k + 1}), {secs:.3f} s in; at most {speed:.2f} uu/s and {spin:.3f} rad/s before")
    if not frozen:
        return
    dwell = min(f[2] for f in frozen)
    print(f"timer {dwell:.3f} to {max(f[2] for f in frozen):.3f} s; linear threshold above {max(f[0] for f in frozen):.2f} uu/s, "
          f"angular above {max(f[1] for f in frozen):.3f} rad/s")
    for name, rows in awake:
        bound = awake_bound(rows, dwell)
        if bound is None:
            print(f"{name}: never frozen, shorter than {dwell:.3f} s; bounds nothing")
            continue
        print(f"{name}: never frozen; its slowest {dwell:.3f} s window peaks at {bound[0]:.2f} uu/s and {bound[1]:.3f} rad/s, "
              f"so linear threshold at most {bound[0]:.2f} uu/s or angular at most {bound[1]:.3f} rad/s")


def bounces(data):
    """Vertical bounce ratios from the ceiling and floor hits in probe_ball_ceiling."""
    frames = dict(data)["probe_ball_ceiling.jsonl"]
    jumps = sorted(((abs(frames[k]["ball"]["velocity"]["z"] - frames[k - 1]["ball"]["velocity"]["z"]), k)
                    for k in range(1, len(frames))), reverse=True)[:2]
    for _, k in jumps:
        before, after = frames[k - 1]["ball"]["velocity"]["z"], frames[k]["ball"]["velocity"]["z"]
        surface = "ceiling" if frames[k]["ball"]["position"]["z"] > 1000 else "floor"
        print(f"{surface} bounce at frame {k} (file line {k + 1}): vz {before:.2f} -> {after:.2f}, ratio {abs(after / before):.4f} "
              "(adjacent frames, no contact-tick interpolation)")


def pickups(data):
    """First-car small pad pickups by a car on the ground: (speed, file, frame, d_after, d_before).

    The distances are the car's origin to the pad centre in the plane, after the pickup tick and
    the tick before it. That is the quantity RocketSim's first-acquisition test uses (a cylinder
    around the pad against the car origin); it says nothing about the retention test.
    """
    events = []
    for name, frames in data:
        for k in range(3, len(frames)):
            now, before = frames[k]["cars"][0], frames[k - 1]["cars"][0]
            gain = now["boost_amount"] - before["boost_amount"]
            if 0.5 < gain < 13 and now["position"]["z"] < 20 and before["position"]["z"] < 20:
                pad = min(SMALL_PADS, key=lambda q: math.hypot(q[0] - now["position"]["x"], q[1] - now["position"]["y"]))
                dist = lambda c: math.hypot(c["position"]["x"] - pad[0], c["position"]["y"] - pad[1])
                speed = math.hypot(now["velocity"]["x"], now["velocity"]["y"])
                events.append((speed, name, k, dist(now), dist(before)))
    return sorted(events)


def pads(data):
    """Origin-to-pad distance at small pad pickups, against RocketSim's 144 uu and the port's 176 uu."""
    events = pickups(data)
    print(f"ground small-pad pickups: {len(events)}")
    for speed, name, k, after, before in events[:3]:
        print(f"  {name} frame {k}: {speed:.0f} uu/s, origin {after:.1f} uu from the pad after the pickup, {before:.1f} the tick before")
    slow = events[:2]
    print(f"two slowest approaches bracket the origin distance at the trigger to [{max(e[3] for e in slow):.1f}, {min(e[4] for e in slow):.1f})")
    speed, name, k, after, _ = events[0]
    frames = dict(data)[name]
    pad = min(SMALL_PADS, key=lambda q: math.hypot(q[0] - frames[k]["cars"][0]["position"]["x"], q[1] - frames[k]["cars"][0]["position"]["y"]))
    later = next((m for m in range(k, len(frames)) if math.hypot(
        frames[m]["cars"][0]["position"]["x"] - pad[0], frames[m]["cars"][0]["position"]["y"] - pad[1]) < RS_CYL_RAD_SMALL), None)
    if later is not None:
        print(f"  {name}: the origin crosses {RS_CYL_RAD_SMALL} uu at frame {later} (file line {later + 1}), {later - k} frames after the pickup")
    print(f"first acquisition: RocketSim BoostPad.cpp uses {RS_CYL_RAD_SMALL} uu (origin, cylinder); the port fits {PORT_SMALL_PAD_RADIUS} uu (ADR-0072)")
    print(f"pickups with the origin beyond {RS_CYL_RAD_SMALL} uu after the tick: "
          f"{sum(1 for e in events if e[3] > RS_CYL_RAD_SMALL)} of {len(events)}")


COMMANDS = {"caps": caps, "ball-spin": ball_spin, "boost-taps": boost_taps,
            "ball-roll": ball_roll, "ball-rest": ball_rest, "bounces": bounces, "pads": pads}

if __name__ == "__main__":
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    if which != "all" and which not in COMMANDS:
        sys.exit(f"unknown check {which!r}; one of: all, {', '.join(COMMANDS)}")
    data = captures()
    for name in COMMANDS if which == "all" else [which]:
        print(f"== {name}")
        COMMANDS[name](data)
