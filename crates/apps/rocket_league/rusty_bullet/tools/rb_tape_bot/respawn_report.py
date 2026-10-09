"""Where and when a demolished car comes back (PARITY-PLAN workstream D, O018).

    python -I tools/rb_tape_bot/respawn_report.py replays/batch_<stamp> [name-prefix]

For every `<name>_run<k>.jsonl` in which a car (player_id >= 1) drops out of the capture
and returns, prints when it was demolished (seconds from the start), how long it was out,
and its state in the first frame back: position, heading (yaw, degrees), velocity, boost.
The game's capture drops a demolished car, so the gap is the respawn delay.
"""
import json
import math
import pathlib
import re
import sys


def yaw_degrees(q: dict) -> float:
    """Heading of a car from its quaternion (rotation about z)."""
    x, y, z, w = q["x"], q["y"], q["z"], q["w"]
    return math.degrees(math.atan2(2 * (w * z + x * y), 1 - 2 * (y * y + z * z)))


def report(path: pathlib.Path) -> list[str]:
    frames = [json.loads(line) for line in path.open(encoding="utf-8")]
    lines = []
    present: dict[int, bool] = {}
    out_since: dict[int, float] = {}
    last_seen: dict[int, dict] = {}
    for frame in frames:
        t = frame["timestamp_secs"]
        ids = {car["player_id"]: car for car in frame["cars"]}
        for pid in set(present) | set(ids):
            here = pid in ids
            was = present.get(pid)
            if was is None:
                present[pid] = here
                if here:
                    last_seen[pid] = ids[pid]
                continue
            if was and not here:
                out_since[pid] = t
            elif not was and here and pid in out_since:
                car = ids[pid]
                p, v = car["position"], car["velocity"]
                lines.append(
                    f"  car {pid}: out at {out_since[pid]:.3f} s, back at {t:.3f} s "
                    f"(gap {t - out_since[pid]:.3f} s) at ({p['x']:.0f}, {p['y']:.0f}, {p['z']:.1f}) "
                    f"yaw {yaw_degrees(car['rotation']):.1f} deg, speed {math.sqrt(v['x'] ** 2 + v['y'] ** 2 + v['z'] ** 2):.1f}, "
                    f"boost {car['boost_amount']:.1f}; last seen at ({last_seen[pid]['position']['x']:.0f}, {last_seen[pid]['position']['y']:.0f})"
                )
                del out_since[pid]
            present[pid] = here
            if here:
                last_seen[pid] = ids[pid]
    return lines


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    batch = pathlib.Path(sys.argv[1])
    prefix = sys.argv[2] if len(sys.argv) > 2 else ""
    for capture in sorted(batch.glob("*_run*.jsonl")):
        if capture.name.endswith(".pads.jsonl") or not re.search(r"_run\d+\.jsonl$", capture.name):
            continue
        if not capture.name.startswith(prefix):
            continue
        lines = report(capture)
        print(capture.name)
        print("\n".join(lines) if lines else "  no car left the capture")
    return 0


if __name__ == "__main__":
    sys.exit(main())
