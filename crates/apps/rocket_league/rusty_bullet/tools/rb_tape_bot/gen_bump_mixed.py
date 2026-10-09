"""Airborne bumper into a grounded victim (PARITY-PLAN workstream D, RB-RESEARCH-O018).

    python -I tools/rb_tape_bot/gen_bump_mixed.py

Writes experiments/bumpag_<speed>_<height>.json: a level attacker set `height` uu up (its
wheels off the ground, above their 31 uu reach) and moving at `speed` uu/s, with no input,
reaches a stopped teammate on the ground about 0.1 s later, still airborne (it falls a few
uu by then). The bump rules were fitted with both cars in the air (`bumpa_air_*`) or the
bumper on the ground; the one sample of this mixed case (`tri_chain`) shows the victim
thrown up, not down. Prints the names.
"""
import json
import math
import pathlib

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
# The attacker's nose reaches the victim when their centres are 118 uu apart.
CONTACT_DISTANCE = 118
LEAD_SECS = 0.1
TAPES = [(700, 45), (1000, 45), (1400, 45), (1800, 45), (1000, 38), (1400, 52)]


def car(y: float, z: float, vy: float) -> dict:
    return {
        "location": [0, y, z],
        "rotation": [0, math.radians(90), 0],
        "velocity": [0, vy, 0],
        "angular_velocity": [0, 0, 0],
        "boost": 0,
    }


def tape(speed: int, height: int) -> dict:
    start = -(CONTACT_DISTANCE + LEAD_SECS * speed)
    return {
        "name": f"bumpag_{speed}_{height}: level attacker {height} uu up at {speed} uu/s coasts into a stopped grounded teammate",
        "settle_ticks": 0,
        "car": car(start, height, speed),
        "ball": {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": [{"ticks": 150}],
        "others": [{"car": car(0, 17, 0), "steps": [{"ticks": 150}], "team": 0}],
    }


def main() -> None:
    names = []
    for speed, height in TAPES:
        name = f"bumpag_{speed}_{height}"
        text = json.dumps(tape(speed, height), indent=2) + chr(10)
        (OUT / f"{name}.json").write_text(text, encoding="utf-8", newline=chr(10))
        names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
