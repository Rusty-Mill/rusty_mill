"""Single ball events and how well the port predicts each (PARITY-PLAN workstream B step 1).

    python -I tools/rb_tape_bot/ball_events.py replays/batch_<stamp> [name-prefix] [threshold]

For each capture runs `rb-verify --seed-first-frame --self-onestep` (the port stepped once
from each recorded frame, so a row's error is that tick's model error alone), clusters the
ticks whose ball velocity error passes `threshold` (default 5 uu/s) into events, and prints the
worst tick of each event with the car-ball distance, the ball's recorded speed change and the
car's input. Events are sorted worst first across the batch; a summary gives how many
events pass each size.
"""
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
VERIFY = ROOT / "target" / "release" / "rb-verify.exe"
LINE = re.compile(
    r"t=\s*([\d.]+)s car=0 \| thr\s+([-\d.]+) str\s+([-\d.]+) p\s+([-\d.]+) y\s+([-\d.]+) r\s+([-\d.]+) (\S+) \| "
    r"err vel\s+([\d.]+) spin\s+([\d.]+) ball\s+([\d.]+)"
)
CLUSTER_GAP_SECS = 0.15


def ticks_of(capture: pathlib.Path) -> list[tuple[float, float, float, str]]:
    out = subprocess.run(
        [str(VERIFY), "--seed-first-frame", "--self-onestep", str(capture), "100000"],
        capture_output=True,
        text=True,
    ).stdout
    rows = []
    for line in out.splitlines():
        m = LINE.match(line)
        if m:
            rows.append((float(m.group(1)), float(m.group(8)), float(m.group(10)), m.group(7)))
    return rows


def car_ball_distance(frames: list[dict], t: float) -> float:
    best = min(frames, key=lambda f: abs(f["timestamp_secs"] - t))
    car = best["cars"][0]["position"]
    ball = best["ball"]["position"]
    return sum((car[a] - ball[a]) ** 2 for a in "xyz") ** 0.5


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    batch = pathlib.Path(sys.argv[1])
    prefix = sys.argv[2] if len(sys.argv) > 2 and not sys.argv[2].replace(".", "").isdigit() else ""
    threshold = float(sys.argv[-1]) if sys.argv[-1].replace(".", "").isdigit() and len(sys.argv) > 2 else 5.0
    events = []
    for capture in sorted(batch.glob("*_run1.jsonl")):
        if not capture.name.startswith(prefix):
            continue
        frames = [json.loads(line) for line in capture.open(encoding="utf-8")]
        rows = [r for r in ticks_of(capture) if r[0] > 0.3 and r[2] > threshold]
        cluster: list[tuple[float, float, float, str]] = []
        for row in rows + [None]:  # type: ignore[list-item]
            if cluster and (row is None or row[0] - cluster[-1][0] > CLUSTER_GAP_SECS):
                worst = max(cluster, key=lambda r: r[2])
                events.append((worst[2], capture.name[:-11], worst[0], worst[1], worst[3], car_ball_distance(frames, worst[0]), len(cluster)))
                cluster = []
            if row is not None:
                cluster.append(row)
    events.sort(reverse=True)
    print(f"{len(events)} events over {threshold} uu/s ball velocity error")
    for sizes in (5, 20, 50, 100, 200):
        print(f"  over {sizes:>3} uu/s: {sum(1 for e in events if e[0] > sizes)}")
    print(f"{'ball err':>9} {'tape':<14}{'t':>7}{'car err':>9}  flags  {'car-ball dist':>13} {'ticks':>5}")
    for e in events[:25]:
        print(f"{e[0]:9.1f} {e[1]:<14}{e[2]:7.3f}{e[3]:9.1f}  {e[4]:<5}  {e[5]:13.1f} {e[6]:5d}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
