"""Tapes to test the game's 8-bit input quantization on the air-control axes.

    python -I tools/rb_tape_bot/gen_quant_probe.py

The game round-trips every analog input through a byte before the car sees it: negative values step
by 1/128, positive by 1/127, so +0.5 becomes 64/127 = 0.5039 while -0.5 stays -0.5 (the port does the
same in `drive::quantize_input`). A level car at rest at z = 1000 idles
60 ticks (past the fresh-car pitch lock), then holds one axis at +0.5 or -0.5 for 30 ticks. Air control is
linear in the input over that span, so the spin about the held axis after N ticks should differ between the
two signs by the same factor: |w(+0.5)| / |w(-0.5)| = 1.0079 if the game quantizes, 1.0000 if not.
Measured: 1.0079 on roll, 1.008 to 1.009 on pitch and yaw, no run-to-run spread.
Writes experiments/quant_<axis>_<pos|neg>.json (six tapes) and prints the names; run each with
`run_batch.ps1 -Repeat 3 -Scenarios <names>`, then `quant_ratio.py <batch dir>`.
"""
import json
import pathlib

EXP = pathlib.Path(__file__).resolve().parent / "experiments"
IDLE_TICKS = 60
HOLD_TICKS = 30
AXES = ("pitch", "yaw", "roll")


def tape(axis: str, value: float) -> dict:
    sign = "pos" if value > 0 else "neg"
    return {
        "name": f"quant_{axis}_{sign}: level at z = 1000, {axis} {value} after {IDLE_TICKS} idle ticks",
        "settle_ticks": 0,
        "car": {
            "location": [0, 0, 1000],
            "rotation": [0, 0, 0],
            "velocity": [0, 0, 0],
            "angular_velocity": [0, 0, 0],
            "boost": 0,
        },
        "ball": {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": [{"ticks": IDLE_TICKS}, {"ticks": HOLD_TICKS, axis: value}],
    }


def main() -> None:
    names = []
    for axis in AXES:
        for value in (0.5, -0.5):
            doc = tape(axis, value)
            name = f"quant_{axis}_{'pos' if value > 0 else 'neg'}"
            (EXP / f"{name}.json").write_text(json.dumps(doc, indent=2) + chr(10), encoding="utf-8", newline=chr(10))
            names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
