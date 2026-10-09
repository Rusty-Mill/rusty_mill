"""Demolition tapes for the respawn rule (PARITY-PLAN workstream D, RB-RESEARCH-O018).

    python -I tools/rb_tape_bot/gen_demolitions.py

Writes experiments/demo_<n>.json: an attacker boosting at 2300 uu/s runs into a stopped
enemy 808 uu ahead (the `bumpd_2300` geometry), at a different place and heading each
time, and the tape runs 700 ticks (5.8 s) so the victim's respawn, three seconds after
the demolition, is on the recording (it is gone from the capture while it is out).
The attacker runs along +y; `yaw` turns the victim only. Prints the names.
"""
import json
import math
import pathlib

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
TICKS = 700
SPOTS = [
    (-2400, -2000, 90), (0, -2000, 90), (2400, -2000, 90),
    (-2400, 500, 90), (0, 500, 90), (2400, 500, 90),
    (-2400, 2500, 90), (0, 2500, 90), (2400, 2500, 90),
    (0, 0, 0), (1200, -1000, 180), (-1200, 1000, -90),
]


def car(x: float, y: float, yaw_deg: float, velocity: float, boost: int) -> dict:
    return {
        "location": [x, y, 17],
        "rotation": [-0.0096, math.radians(yaw_deg), 0],
        "velocity": [0, velocity, 0],
        "angular_velocity": [0, 0, 0],
        "boost": boost,
    }


def tape(index: int, x: int, y: int, yaw: int) -> dict:
    return {
        "name": f"demo_{index}: boosting attacker demolishes a stopped enemy at ({x}, {y}) facing {yaw} degrees",
        "settle_ticks": 0,
        "car": car(x, y - 808, 90, 2300, 100),
        "ball": {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": [{"ticks": 150, "throttle": 1, "boost": True}, {"ticks": TICKS - 150}],
        "others": [{"car": car(x, y, yaw, 0, 0), "steps": [{"ticks": TICKS}], "team": 1}],
    }


def main() -> None:
    names = []
    for index, (x, y, yaw) in enumerate(SPOTS):
        name = f"demo_{index}"
        text = json.dumps(tape(index, x, y, yaw), indent=2) + chr(10)
        (OUT / f"{name}.json").write_text(text, encoding="utf-8", newline=chr(10))
        names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
