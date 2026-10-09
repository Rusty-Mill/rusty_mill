"""Wall approaches with the throttle released (PARITY-PLAN workstream A, RB-RESEARCH-O012).

    python -I tools/rb_tape_bot/gen_wall_coast.py

Writes experiments/wallgc_<angle>_<speed>.json and its mirror wallgcm_<angle>_<speed>.json
from `wallg_<angle>_<speed>.json`: the same start, but with no input at all, so the car
coasts up the wall on whatever speed it arrives with. That takes the engine, the
boost and the sticky force's throttle term out of the climb (only the suspension, the
tires' coasting brake, the sticky force and gravity are left). The mirror puts it on the
x = -4096 wall with its left side leading. Prints the names.
"""
import json
import math
import pathlib

EXP = pathlib.Path(__file__).resolve().parent / "experiments"
CASES = [(30, 1800), (30, 1200), (60, 1800), (60, 1200)]
TICKS = 420


def mirror(tape: dict, name: str) -> dict:
    car = dict(tape["car"])
    car["location"] = [-car["location"][0], car["location"][1], car["location"][2]]
    car["velocity"] = [-car["velocity"][0], car["velocity"][1], car["velocity"][2]]
    pitch, yaw, roll = car["rotation"]
    car["rotation"] = [pitch, math.pi - yaw, -roll]
    return {**tape, "name": name, "car": car}


def write(tape: dict, name: str) -> None:
    text = json.dumps(tape, indent=2) + chr(10)
    (EXP / f"{name}.json").write_text(text, encoding="utf-8", newline=chr(10))


def main() -> None:
    names = []
    for angle, speed in CASES:
        source = json.loads((EXP / f"wallg_{angle}_{speed}.json").read_text(encoding="utf-8"))
        coast = {**source, "name": f"wallgc_{angle}_{speed}: wall approach at {angle} degrees, {speed} uu/s, throttle released", "steps": [{"ticks": TICKS}]}
        write(coast, f"wallgc_{angle}_{speed}")
        write(mirror(coast, f"wallgcm_{angle}_{speed}: mirror of wallgc_{angle}_{speed}"), f"wallgcm_{angle}_{speed}")
        names += [f"wallgc_{angle}_{speed}", f"wallgcm_{angle}_{speed}"]
    print(",".join(names))


if __name__ == "__main__":
    main()
