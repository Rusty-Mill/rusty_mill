"""Reads the quantization probe captures: is |w(+0.5)| / |w(-0.5)| 1.0079 (quantized) or 1.0000?

    python -I tools/rb_tape_bot/quant_ratio.py replays/batch_<stamp>

For each axis, finds the frame where the held input first becomes nonzero in every `quant_<axis>_<pos|neg>_run*`
capture and prints the spin magnitude (rad/s) 5, 10, 20 and 30 frames later, averaged over the runs, and the
pos/neg ratio. Prediction with the game's byte round trip: 64/127 / 0.5 = 1.0079; without it: 1.0000. A
ratio is only meaningful if the per-run spread (printed) is well under 0.3 %.
"""
import json
import math
import pathlib
import statistics
import sys

OFFSETS = (5, 10, 20, 30)
PREDICTED = (64 / 127) / 0.5


def spin_after_press(path: pathlib.Path, axis: str) -> list[float] | None:
    frames = [json.loads(line) for line in path.open(encoding="utf-8")]
    start = next((i for i, f in enumerate(frames) if abs(((f.get("cars") or [{}])[0].get("input") or {}).get(axis, 0)) > 0), None)
    if start is None or start + max(OFFSETS) >= len(frames):
        return None
    out = []
    for k in OFFSETS:
        w = frames[start + k]["cars"][0]["angular_velocity"]
        out.append(math.sqrt(w["x"] ** 2 + w["y"] ** 2 + w["z"] ** 2))
    return out


def main() -> None:
    batch = pathlib.Path(sys.argv[1])
    print(f"predicted ratio with quantization: {PREDICTED:.4f}")
    for axis in ("pitch", "yaw", "roll"):
        runs = {}
        for sign in ("pos", "neg"):
            rows = [spin_after_press(p, axis) for p in sorted(batch.glob(f"quant_{axis}_{sign}_run*.jsonl")) if not p.name.endswith(".pads.jsonl")]
            runs[sign] = [r for r in rows if r]
        if not runs["pos"] or not runs["neg"]:
            print(f"{axis}: missing captures")
            continue
        for i, k in enumerate(OFFSETS):
            pos = [r[i] for r in runs["pos"]]
            neg = [r[i] for r in runs["neg"]]
            spread = max(statistics.pstdev(pos) / statistics.mean(pos), statistics.pstdev(neg) / statistics.mean(neg))
            print(f"{axis:5s} +{k:2d} frames: w(+) {statistics.mean(pos):.4f}  w(-) {statistics.mean(neg):.4f}"
                  f"  ratio {statistics.mean(pos) / statistics.mean(neg):.4f}  spread {spread * 100:.2f}%")


if __name__ == "__main__":
    main()
