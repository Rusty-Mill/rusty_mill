"""Scores a batch of fuzz tapes and says where each one first goes wrong.

    python -I tools/rb_tape_bot/fuzz_report.py replays/batch_<stamp> [threshold_uu]

For every `fuzz_*_run1.jsonl`: the port fed the recorded input
(`rb-verify --recorded-inputs`), the mean and max car error, and the first tick
the error passes `threshold_uu` (default 10) with what the car was doing there
(recorded z, speed, and the input pressed), so a failure can be traced to a
mechanic. A summary line gives the mean over all tapes and how many stay under
the threshold for the whole tape.
"""
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
VERIFY = ROOT / "target" / "release" / "rb-verify.exe"
EXP = ROOT / "tools" / "rb_tape_bot" / "experiments"


def rows_of(scenario: pathlib.Path, capture: pathlib.Path) -> tuple[str, dict]:
    out = subprocess.run(
        [str(VERIFY), "--scenario", str(scenario), "--against", str(capture), "--recorded-inputs", "1"],
        capture_output=True,
        text=True,
    ).stdout
    rows = {}
    pattern = r"\s*(\d+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+\(\s*([-\d.]+),\s*([-\d.]+),\s*([-\d.]+)\)"
    for line in out.splitlines():
        m = re.match(pattern, line)
        if m:
            rows[int(m.group(1))] = (float(m.group(2)), float(m.group(3)), tuple(float(m.group(k)) for k in (5, 6, 7)))
    return out, rows


def main() -> None:
    batch = pathlib.Path(sys.argv[1])
    threshold = float(sys.argv[2]) if len(sys.argv) > 2 else 10.0
    means, clean = [], 0
    for capture in sorted(batch.glob("fuzz_*_run1.jsonl"), key=lambda p: int(p.name.split("_")[1])):
        name = capture.name[: -len("_run1.jsonl")]
        scenario = EXP / f"{name}.json"
        out, rows = rows_of(scenario, capture)
        m = re.search(r"position error: mean ([\d.]+) uu, max ([\d.]+)", out)
        if not m or not rows:
            print(f"{name:<9} unscored: {out.strip().splitlines()[0][:90] if out.strip() else 'no output'}")
            continue
        mean, mx = float(m.group(1)), float(m.group(2))
        means.append(mean)
        first = next((t for t in sorted(rows) if rows[t][0] > threshold), None)
        if first is None:
            clean += 1
            print(f"{name:<9} mean {mean:6.1f} max {mx:6.1f}  stays under {threshold:g} uu")
            continue
        tape = json.loads(scenario.read_text(encoding="utf-8"))
        # The tape input at that tick (the recorded input matches it but for latency).
        tick, remaining, pressed = first, first - tape.get("settle_ticks", 0), "settling"
        if remaining >= 0:
            for st in tape["steps"]:
                if remaining < st["ticks"]:
                    pressed = {k: v for k, v in st.items() if k != "ticks"}
                    break
                remaining -= st["ticks"]
        z = rows[first][2][2]
        print(f"{name:<9} mean {mean:6.1f} max {mx:6.1f}  first over {threshold:g} at tick {tick}: z {z:.0f}, tape {pressed}")
    if means:
        print(f"\n{len(means)} tapes, mean of means {sum(means) / len(means):.1f} uu, {clean} under {threshold:g} uu throughout")


if __name__ == "__main__":
    main()
