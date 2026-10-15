"""Capture-only checks of the two rules rusty_bullet encodes: input quantization and the pedal deadzone.

Quantization. Hypothesis: every analog input is clamped to [-1, 1] and round-tripped through one byte,
``f = v<0 ? max(v*128,-128) : min(v*127,127); byte = floor(f+128+0.5); q = (byte-128)/128 if byte<128 else /127``.
A held +v and -v on roll from a level car at rest give spins in the ratio q(+v)/|q(-v)| (air control is linear in the
input over the 30-tick hold), so the ratio is a function of the level v: 1.0079 at 0.5, but 0.9967 at 0.7 and 0.9991
at 0.9, because the two signs round differently. Each level is measured and compared with three hypotheses:
the byte rule above, no quantization (ratio 1), and a symmetric 127-step rounding (ratio 1).

Deadzone. From a standstill on the floor a throttle of 0.008 (one byte step, 0.0079) and 0.016 (two steps, 0.0157) is held
for 90 ticks; a car that drives at the second but not the first puts the pedal deadzone between them. Because inputs
sit on the byte grid, any threshold in [0.0079, 0.0157) gives identical behaviour.

Usage: ``python -I capture_checks.py BATCH_DIR`` where BATCH_DIR holds the quantlvl_roll_* and dz_* captures.
"""
from __future__ import annotations

import json
import math
import statistics
import sys
from pathlib import Path

OFFSETS = (10, 20, 30)


def q(v: float) -> float:
    v = max(-1.0, min(1.0, v))
    f = max(v * 128, -128.0) if v < 0 else min(v * 127, 127.0)
    byte = math.floor(f + 128 + 0.5)
    return (byte - 128) / 128 if byte < 128 else (byte - 128) / 127


def frames(path: Path) -> list[dict]:
    out = []
    for line in path.open(encoding="utf-8"):
        d = json.loads(line)
        if d.get("cars") and d["cars"][0].get("input"):
            out.append(d["cars"][0])
    return out


def spin_after_press(path: Path, axis: str) -> list[float] | None:
    fr = frames(path)
    start = next((i for i, c in enumerate(fr) if abs(c["input"][axis]) > 0), None)
    if start is None or start + max(OFFSETS) >= len(fr):
        return None
    res = []
    for k in OFFSETS:
        w = fr[start + k]["angular_velocity"]
        res.append(math.sqrt(w["x"] ** 2 + w["y"] ** 2 + w["z"] ** 2))
    return res


def runs(batch: Path, stem: str) -> list[list[float]]:
    rows = []
    for p in sorted(batch.glob(f"{stem}_run*.jsonl")):
        if p.name.endswith(".pads.jsonl"):
            continue
        r = spin_after_press(p, "roll")
        if r:
            rows.append(r)
    return rows


def quantization(batch: Path, existing: Path | None) -> None:
    print("Quantization: |spin(+v)| / |spin(-v)| on roll, mean over runs and the three frame offsets")
    print("level   runs  measured   byte rule   no quantization   symmetric 127")
    for v in (0.1, 0.3, 0.5, 0.7, 0.9):
        if v == 0.5 and existing is not None:
            pos, neg = runs(existing, "quant_roll_pos"), runs(existing, "quant_roll_neg")
        else:
            pct = round(v * 100)
            pos, neg = runs(batch, f"quantlvl_roll_{pct}_pos"), runs(batch, f"quantlvl_roll_{pct}_neg")
        if not pos or not neg:
            print(f"{v:4.1f}   missing captures")
            continue
        ratios = [statistics.mean(r[i] for r in pos) / statistics.mean(r[i] for r in neg) for i in range(len(OFFSETS))]
        pred = q(v) / abs(q(-v))
        print(f"{v:4.1f}   {len(pos)}+{len(neg)}   {statistics.mean(ratios):.4f}    {pred:.4f}      1.0000            1.0000"
              f"      (offsets: {', '.join(f'{r:.4f}' for r in ratios)})")


def deadzone(batch: Path) -> None:
    print("\nDeadzone: forward speed (uu/s) along the car's heading, mean over runs")
    print("tape            throttle sent  byte value   speed at +30 / +60 / +90 frames")
    for name, sent in (("dz_0", 0.0), ("dz_1step", 0.008), ("dz_2step", 0.016), ("dz_neg1step", -0.008)):
        speeds = []
        for p in sorted(batch.glob(f"{name}_run*.jsonl")):
            if p.name.endswith(".pads.jsonl"):
                continue
            fr = frames(p)
            start = next((i for i, c in enumerate(fr) if abs(c["input"]["throttle"]) > 0), None)
            if start is None:
                start = max(0, len(fr) - 120)
            row = []
            for k in (30, 60, 90):
                if start + k >= len(fr):
                    break
                v = fr[start + k]["velocity"]
                row.append(math.sqrt(v["x"] ** 2 + v["y"] ** 2) * (1 if v["y"] >= 0 else -1))
            if len(row) == 3:
                speeds.append(row)
        if not speeds:
            print(f"{name:14s}  no usable captures")
            continue
        m = [statistics.mean(s[i] for s in speeds) for i in range(3)]
        print(f"{name:14s}  {sent:+.3f}         {q(sent):+.4f}     {m[0]:+7.2f} / {m[1]:+7.2f} / {m[2]:+7.2f}   ({len(speeds)} runs)")


def main() -> None:
    batch = Path(sys.argv[1])
    existing = Path(sys.argv[2]) if len(sys.argv) > 2 else None
    quantization(batch, existing)
    deadzone(batch)


if __name__ == "__main__":
    main()
