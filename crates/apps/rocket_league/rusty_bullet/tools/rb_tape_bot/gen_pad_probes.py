"""Boost pad pickup probes (PARITY-PLAN workstream C step 2).

    python -I tools/rb_tape_bot/gen_pad_probes.py

Writes experiments/padp_*.json: a car with no boost passes one pad at a known
lateral offset (or drops onto it from 400 uu), so the pad log
(`<capture>.pads.jsonl`, ADR-0071) says at which offset and height the pad is
taken. Run them with `run_batch.ps1 -Repeat 1 -Scenarios <names>` (the script
prints the list).

* padp_s_<d>: small pad 14 at (0, -1024), the car drives +y at 900 uu/s with
  its origin d uu to the side (nothing else is within 1000 uu of the path);
* padp_b_<d>: big pad 18 at (3584, 0), same, d uu toward the field centre;
* padp_drop_<d>: the car falls from 400 uu onto small pad 14, d uu to the side.
"""
import json
import pathlib

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
SMALL = (0, -1024)
BIG = (3584, 0)
SIDE = list(range(0, 261, 20))
BIG_SIDE = list(range(0, 301, 60))
DROP_SIDE = [0, 60, 120, 180]
YAW_PLUS_Y = 1.5707963


def scenario(name: str, loc: list, vel: list, ticks: int) -> dict:
    return {
        "name": name,
        "settle_ticks": 60,
        "car": {"location": loc, "rotation": [-0.0096, YAW_PLUS_Y, 0], "velocity": vel,
                "angular_velocity": [0, 0, 0], "boost": 0},
        "ball": {"location": [-3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": [{"ticks": ticks, "throttle": 1}],
    }


def write(tape: dict, name: str) -> None:
    (OUT / f"{name}.json").write_text(json.dumps(tape, indent=2) + "\n", encoding="utf-8", newline="\n")


def main() -> None:
    names = []
    for d in SIDE:
        name = f"padp_s_{d}"
        tape = scenario(f"{name}: small pad 14, car {d} uu to the side, 900 uu/s", [SMALL[0] + d, -1700, 17], [0, 900, 0], 150)
        write(tape, name)
        names.append(name)
    for d in BIG_SIDE:
        name = f"padp_b_{d}"
        tape = scenario(f"{name}: big pad 18, car {d} uu toward the centre, 900 uu/s", [BIG[0] - d, -700, 17], [0, 900, 0], 170)
        write(tape, name)
        names.append(name)
    for d in DROP_SIDE:
        name = f"padp_drop_{d}"
        tape = scenario(f"{name}: car falls from 400 uu onto small pad 14, {d} uu to the side", [SMALL[0] + d, SMALL[1], 400], [0, 0, 0], 100)
        tape["steps"] = [{"ticks": 100}]
        write(tape, name)
        names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
