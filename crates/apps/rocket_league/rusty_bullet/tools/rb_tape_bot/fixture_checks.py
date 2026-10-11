"""Read physics constants straight off the committed game captures (PHYSICS-PARITY pass 2).

    python -I tools/rb_tape_bot/fixture_checks.py [all|caps|ball-spin|boost-taps|ball-roll|bounces|pads]

Reads `fixtures/*.jsonl` (BakkesMod state and recorded inputs, 120 Hz) with the standard library
only and prints the numbers `docs/research/PHYSICS-PARITY-VS-ROCKETSIM.md` quotes. Nothing here
runs the port; the captures are the game's own, so each result is a measurement of the game.
"""
import json
import math
import pathlib
import statistics
import sys

FIXTURES = pathlib.Path(__file__).resolve().parent / "fixtures"
DT = 1 / 120
BALL_DRAG = 0.03  # per second; removed before reading friction (RocketSim BALL_DRAG, ADR-0013)
# Octane hitbox half-extents and offset, as the port uses them (RocketSim CarConfig.cpp).
HALF = (60.2535, 43.3497, 19.32955)
OFFSET = (13.8757, 0.0, 20.755)
# The 28 small boost pad centres (x, y), mirrored; RocketSim RLConst.h LOCS_SMALL_SOCCAR.
SMALL_PADS = [
    (0, -4240), (-1792, -4184), (1792, -4184), (-940, -3308), (940, -3308), (0, -2816),
    (-3584, -2484), (3584, -2484), (-1788, -2300), (1788, -2300), (-2048, -1036), (0, -1024),
    (2048, -1036), (-1024, 0), (1024, 0), (-2048, 1036), (0, 1024), (2048, 1036),
    (-1788, 2300), (1788, 2300), (-3584, 2484), (3584, 2484), (0, 2816), (-940, 3308),
    (940, 3308), (-1792, 4184), (1792, 4184), (0, 4240),
]


def load(path):
    frames = []
    for line in path.read_text().splitlines():
        if line.strip():
            try:
                frames.append(json.loads(line))
            except json.JSONDecodeError:
                pass  # a crash-truncated last line (ADR-0005)
    return frames


def captures():
    return [(p.name, load(p)) for p in sorted(FIXTURES.glob("*.jsonl"))]


def norm(v):
    return math.sqrt(v["x"] ** 2 + v["y"] ** 2 + v["z"] ** 2)


def caps(data):
    """Largest ball angular velocity, ball speed, car speed and car angular velocity seen."""
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


def ball_spin(data):
    """How many frames sit at the ball's angular velocity cap, and whether any exceed it."""
    near = [(n, f) for n, fr in data for f in fr if abs(norm(f["ball"]["angular_velocity"]) - 6.0) < 1e-3]
    above = sum(1 for _, fr in data for f in fr if norm(f["ball"]["angular_velocity"]) > 6.0001)
    print(f"frames within 0.001 of 6.0 rad/s: {len(near)} in {len({n for n, _ in near})} captures; above 6.0001: {above}")


def boost_taps(data):
    """Short boost button holds and the ticks on which the tank actually drops afterwards."""
    for name, frames in data:
        i = 0
        series = [(f["cars"][0]["input"]["boost"], f["cars"][0]["boost_amount"]) for f in frames if f["cars"]]
        while i < len(series):
            if not series[i][0]:
                i += 1
                continue
            j = i
            while j < len(series) and series[j][0]:
                j += 1
            drops = [k - i for k in range(i, min(j + 16, len(series) - 1)) if series[k + 1][1] < series[k][1] - 1e-6]
            if j - i <= 14:
                print(f"{name}: held {j - i} frames; {len(drops)} drops at offsets +{drops[0] + 1}..+{drops[-1] + 1}, "
                      f"{series[i][1]:.2f} -> {series[min(j + 16, len(series) - 1)][1]:.2f}")
            i = j


def ball_roll(data):
    """Sliding friction, spin growth and the speed at which sliding ends (probe_ball_roll)."""
    frames = dict(data)["probe_ball_roll.jsonl"]
    rows = [(f["timestamp_secs"], f["ball"]["velocity"]["x"], norm(f["ball"]["angular_velocity"]), f["ball"]["position"]["z"])
            for f in frames]
    est = [-(rows[i + 1][1] - rows[i - 1][1]) / (rows[i + 1][0] - rows[i - 1][0]) - BALL_DRAG * rows[i][1]
           for i in range(1, len(rows) - 1) if rows[i][1] > 600 and abs(rows[i][3] - 93.2) < 0.2]
    print(f"sliding friction (deceleration minus drag): {statistics.mean(est):.1f} uu/s2, sd {statistics.pstdev(est):.1f}, n={len(est)}")
    t0 = next(i for i, r in enumerate(rows) if r[0] >= 0.3)
    t1 = next(i for i, r in enumerate(rows) if r[0] >= 0.8)
    print(f"spin growth: {(rows[t1][2] - rows[t0][2]) / (rows[t1][0] - rows[t0][0]):.3f} rad/s2")
    capped = next(i for i, r in enumerate(rows) if r[2] > 5.9999)
    rolling = next(i for i in range(capped, len(rows)) if rows[i][2] < 5.999)  # spin starts tracking v / radius
    print(f"rolling begins at v = {rows[rolling][1]:.1f} uu/s, spin {rows[rolling][2]:.3f}, "
          f"v / spin = {rows[rolling][1] / rows[rolling][2]:.1f} (6 x 91.25 = {6 * 91.25})")


def bounces(data):
    """Vertical bounce ratios from the ceiling and floor hits in probe_ball_ceiling."""
    frames = dict(data)["probe_ball_ceiling.jsonl"]
    jumps = sorted(((abs(frames[k]["ball"]["velocity"]["z"] - frames[k - 1]["ball"]["velocity"]["z"]), k)
                    for k in range(1, len(frames))), reverse=True)[:2]
    for _, k in jumps:
        before, after = frames[k - 1]["ball"]["velocity"]["z"], frames[k]["ball"]["velocity"]["z"]
        print(f"bounce at frame {k}: vz {before:.0f} -> {after:.0f}, ratio {abs(after / before):.4f}")


def rot(q):
    x, y, z, w = (q[c] for c in "xyzw")
    n = math.sqrt(x * x + y * y + z * z + w * w)
    x, y, z, w = x / n, y / n, z / n, w / n
    return [[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
            [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
            [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]]


def footprint(car):
    """The car's hitbox projected on the ground plane, as a convex polygon."""
    r, p = rot(car["rotation"]), car["position"]
    centre = [p[a] + sum(r[i][j] * OFFSET[j] for j in range(3)) for i, a in enumerate("xyz")]
    pts = {(round(centre[0] + sum(r[0][j] * s[j] * HALF[j] for j in range(3)), 6),
            round(centre[1] + sum(r[1][j] * s[j] * HALF[j] for j in range(3)), 6))
           for s in [(a, b, c) for a in (-1, 1) for b in (-1, 1) for c in (-1, 1)]}

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    half = []
    for chain in (sorted(pts), sorted(pts, reverse=True)):
        h = []
        for pt in chain:
            while len(h) >= 2 and cross(h[-2], h[-1], pt) <= 0:
                h.pop()
            h.append(pt)
        half += h[:-1]
    return half


def distance(poly, pad):
    """Shortest distance from the pad centre to the polygon (0 when inside)."""
    pts = [(a - pad[0], b - pad[1]) for a, b in poly]
    edges = list(zip(pts, pts[1:] + pts[:1]))
    if all((b[0] - a[0]) * (0 - a[1]) - (b[1] - a[1]) * (0 - a[0]) >= 0 for a, b in edges):
        return 0.0
    return min(math.hypot(a[0] + (b[0] - a[0]) * t / 400, a[1] + (b[1] - a[1]) * t / 400)
               for a, b in edges for t in range(401))


def pads(data):
    """Small boost pad pickups by a car on the ground, and how close its hitbox was to the pad."""
    events = []
    for name, frames in data:
        for k in range(3, len(frames)):
            now, before = frames[k]["cars"][0], frames[k - 1]["cars"][0]
            gain = now["boost_amount"] - before["boost_amount"]
            if 0.5 < gain < 13 and now["position"]["z"] < 20 and before["position"]["z"] < 20:
                pad = min(SMALL_PADS, key=lambda q: (q[0] - now["position"]["x"]) ** 2 + (q[1] - now["position"]["y"]) ** 2)
                speed = math.hypot(now["velocity"]["x"], now["velocity"]["y"])
                events.append((speed, name, k, distance(footprint(now), pad), distance(footprint(before), pad)))
    print(f"ground small-pad pickups: {len(events)}")
    for speed, name, k, after, before in sorted(events)[:3]:
        print(f"  {name} frame {k}: {speed:.0f} uu/s, hitbox {after:.1f} uu from the pad after the pickup, {before:.1f} the tick before")
    slow = sorted(events)[:2]
    print(f"two slowest approaches bracket the trigger distance to [{max(e[3] for e in slow):.1f}, {min(e[4] for e in slow):.1f})")


COMMANDS = {"caps": caps, "ball-spin": ball_spin, "boost-taps": boost_taps,
            "ball-roll": ball_roll, "bounces": bounces, "pads": pads}

if __name__ == "__main__":
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    if which != "all" and which not in COMMANDS:
        sys.exit(f"unknown check {which!r}; one of: all, {', '.join(COMMANDS)}")
    data = captures()
    for name in COMMANDS if which == "all" else [which]:
        print(f"== {name}")
        COMMANDS[name](data)
