"""Ceiling probes (PARITY-PLAN workstream A.4, RB-RESEARCH-O017/O019).

    python -I tools/rb_tape_bot/gen_ceiling.py

Writes experiments/ceil_<name>.json. The standard arena's ceiling is at z = 2044. Until now
the only ceiling recording was a floor landing (`probe_ceiling`) and the climbs of the
wall approaches that end on it. These put the car on the ceiling directly:

* ceil_up: nose straight up at 1200 uu/s from z = 1500, a nose-first hit;
* ceil_60: nose 60 degrees up and moving along +y, a glancing nose hit;
* ceil_drive / ceil_coast: upside down on the ceiling (roll 180 degrees, 24 uu below it)
  at 1000 uu/s along +y, with the throttle held or released;
* ceil_slow: upside down and slow (300 uu/s) with the throttle held, to see the sticky
  force hold it.

Prints the names.
"""
import json
import math
import pathlib

EXP = pathlib.Path(__file__).resolve().parent / "experiments"


def tape(name: str, text: str, location, rotation, velocity, steps) -> dict:
    return {
        "name": f"{name}: {text}",
        "settle_ticks": 0,
        "car": {
            "location": location,
            "rotation": rotation,
            "velocity": velocity,
            "angular_velocity": [0, 0, 0],
            "boost": 0,
        },
        "ball": {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": steps,
    }


def tapes() -> dict:
    yaw = math.radians(90)
    return {
        "ceil_up": tape("ceil_up", "nose straight up at 1200 uu/s from z = 1500, no input",
                        [0, 0, 1500], [1.5, yaw, 0], [0, 0, 1200], [{"ticks": 240}]),
        "ceil_60": tape("ceil_60", "nose 60 degrees up, 700 uu/s along +y and 1200 up, no input",
                        [0, -1500, 1300], [math.radians(60), yaw, 0], [0, 700, 1200], [{"ticks": 300}]),
        "ceil_drive": tape("ceil_drive", "upside down on the ceiling at 1000 uu/s, throttle held",
                           [0, -2500, 2020], [0, yaw, math.pi], [0, 1000, 0], [{"ticks": 300, "throttle": 1}]),
        "ceil_coast": tape("ceil_coast", "upside down on the ceiling at 1000 uu/s, no input",
                           [0, -2500, 2020], [0, yaw, math.pi], [0, 1000, 0], [{"ticks": 300}]),
        "ceil_slow": tape("ceil_slow", "upside down on the ceiling at 300 uu/s, throttle held",
                          [0, -2500, 2020], [0, yaw, math.pi], [0, 300, 0], [{"ticks": 300, "throttle": 1}]),
    }


def main() -> None:
    names = []
    for key, scenario in tapes().items():
        (EXP / f"{key}.json").write_text(json.dumps(scenario, indent=2) + chr(10), encoding="utf-8", newline=chr(10))
        names.append(key)
    print(",".join(names))


if __name__ == "__main__":
    main()
