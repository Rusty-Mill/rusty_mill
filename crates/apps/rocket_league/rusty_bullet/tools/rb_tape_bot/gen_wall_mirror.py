"""Mirror images of the `wallg_*` wall approaches, onto the other side wall.

    python -I tools/rb_tape_bot/gen_wall_mirror.py

Reads experiments/wallg_<angle>_<speed>.json (a car drives up the x = +4096 wall) and
writes experiments/wallgm_<angle>_<speed>.json with x -> -x and the heading mirrored
(yaw -> pi - yaw), so the car climbs the x = -4096 wall with its left side leading
instead of its right. The arena is mirror-symmetric, so any difference between the
two recordings is the game's wheels (order, side), not the geometry
(PARITY-PLAN workstream A; RB-RESEARCH-O012, O019). Prints the scenario names.
"""
import glob
import json
import math
import pathlib

EXP = pathlib.Path(__file__).resolve().parent / "experiments"


def mirror(tape: dict, name: str) -> dict:
    car = dict(tape["car"])
    car["location"] = [-car["location"][0], car["location"][1], car["location"][2]]
    car["velocity"] = [-car["velocity"][0], car["velocity"][1], car["velocity"][2]]
    pitch, yaw, roll = car["rotation"]
    car["rotation"] = [pitch, math.pi - yaw, -roll]
    return {**tape, "name": f"{name}: mirror of {tape['name']}", "car": car}


def main() -> None:
    names = []
    for path in sorted(glob.glob(str(EXP / "wallg_*.json"))):
        source = pathlib.Path(path)
        name = source.stem.replace("wallg_", "wallgm_")
        out = mirror(json.loads(source.read_text(encoding="utf-8")), name)
        (EXP / f"{name}.json").write_text(json.dumps(out, indent=2) + "\n", encoding="utf-8", newline="\n")
        names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
