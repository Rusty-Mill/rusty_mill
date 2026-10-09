"""Boost pad pickup probes (PARITY-PLAN workstream C step 2).

    python -I tools/rb_tape_bot/gen_pad_probes.py

Writes experiments/padp_*.json: a car with no boost passes one pad at a known
lateral offset (or drops onto it from 400 uu), so the pad log
(`<capture>.pads.jsonl`, ADR-0071) says at which offset and height the pad is
taken. The 5 s wait matters: the state-set teleport picks up every pad on the straight
line from the previous tape's end to the new start (a taken small pad is back
after 4 s, a big one after 10 s), which hid the first batch's pickups.
Run them with `run_batch.ps1 -Repeat 1 -Scenarios <names>` (the script
prints the list).

* padp_s_<d>: small pad 14 at (0, -1024), the car starts at rest 676 uu short of
  it, waits 5 s, then drives +y at half throttle with its origin d uu to the
  side (nothing else is within 1000 uu of the path);
* padp_b_<d>: big pad 18 at (3584, 0), same, d uu toward the field centre;
* padp_drop_<d>: the car falls from 400 uu onto small pad 14, d uu to the side;
* padp_full_<boost>: pass small pad 14 head-on starting with that much boost
  (does a full tank consume the pad, and is the tank capped at 100?);
* padp_fullbig_<boost>: the same for big pad 18;
* padp_bdrop_<d>: the car falls from 400 uu onto big pad 18, d uu to the side.
"""
import json
import pathlib

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
SMALL = (0, -1024)
BIG = (3584, 0)
SIDE = [0, 40, 80, 100, 120, 140, 150, 160, 170, 180, 200, 220, 260]
BIG_SIDE = [0, 120, 180, 200, 210, 220, 240, 300]
DROP_SIDE = [0, 60, 120, 150, 180]
YAW_PLUS_Y = 1.5707963


WAIT_TICKS = 600  # 5 s: past a small pad's 4 s respawn
DRIVE_TICKS = 240


def scenario(name: str, loc: list, vel: list, steps: list) -> dict:
    return {
        "name": name,
        "settle_ticks": 60,
        "car": {"location": loc, "rotation": [-0.0096, YAW_PLUS_Y, 0], "velocity": vel,
                "angular_velocity": [0, 0, 0], "boost": 0},
        "ball": {"location": [-3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": steps,
    }


def write(tape: dict, name: str) -> None:
    (OUT / f"{name}.json").write_text(json.dumps(tape, indent=2) + "\n", encoding="utf-8", newline="\n")


DRIVE = [{"ticks": WAIT_TICKS}, {"ticks": DRIVE_TICKS, "throttle": 0.5}]


FULL_BOOST = [100, 95]
BDROP_SIDE = [0, 100, 180, 200]


def main() -> None:
    names = []
    for d in SIDE:
        name = f"padp_s_{d}"
        tape = scenario(f"{name}: small pad 14, car {d} uu to the side", [SMALL[0] + d, -1700, 17], [0, 0, 0], DRIVE)
        write(tape, name)
        names.append(name)
    for d in BIG_SIDE:
        name = f"padp_b_{d}"
        tape = scenario(f"{name}: big pad 18, car {d} uu toward the centre", [BIG[0] - d, -700, 17], [0, 0, 0], DRIVE)
        write(tape, name)
        names.append(name)
    for d in DROP_SIDE:
        name = f"padp_drop_{d}"
        tape = scenario(f"{name}: car falls from 400 uu onto small pad 14, {d} uu to the side", [SMALL[0] + d, SMALL[1], 400], [0, 0, 0], [{"ticks": 150}])
        write(tape, name)
        names.append(name)
    for amount in FULL_BOOST:
        name = f"padp_full_{amount}"
        tape = scenario(f"{name}: small pad 14 head-on with {amount} boost", [SMALL[0], -1700, 17], [0, 0, 0], DRIVE)
        tape["car"]["boost"] = amount
        write(tape, name)
        names.append(name)
        name = f"padp_fullbig_{amount}"
        tape = scenario(f"{name}: big pad 18 head-on with {amount} boost", [BIG[0], -700, 17], [0, 0, 0], DRIVE)
        tape["car"]["boost"] = amount
        write(tape, name)
        names.append(name)
    for d in BDROP_SIDE:
        name = f"padp_bdrop_{d}"
        tape = scenario(f"{name}: car falls from 400 uu onto big pad 18, {d} uu to the side", [BIG[0] - d, BIG[1], 400], [0, 0, 0], [{"ticks": 150}])
        write(tape, name)
        names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
