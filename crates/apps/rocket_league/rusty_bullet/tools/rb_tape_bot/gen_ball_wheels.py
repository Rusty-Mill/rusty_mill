"""Wheels-first drops onto a resting ball (PARITY-PLAN workstream B step 2, RB-RESEARCH-O014).

    python -I tools/rb_tape_bot/gen_ball_wheels.py

Writes experiments/wob_<offset>_<speed>.json: the ball rests on the floor at the origin,
a level car with no input falls (or is thrown) onto it from just above, `offset` uu
ahead of the ball's centre along the car's nose, at `speed` uu/s downward. The car
lands on the ball with its wheels (the suspension rays reach the ball before the
body does), the case `car_over_ball` mixes with a fast sideways pass. Prints the names.
"""
import json
import pathlib

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
BALL_Z = 93.15
BALL_RADIUS = 91.25
OFFSETS = [0, 50, 100]
SPEEDS = [200, 500, 1000]
TICKS = 180


def tape(offset: int, speed: int) -> dict:
    # Car origin 40 uu above the ball's top: the wheels (about 33 uu below the origin)
    # reach the ball within a tick or two.
    z = BALL_Z + BALL_RADIUS + 40
    return {
        "name": f"wob_{offset}_{speed}: level car {offset} uu off centre falls onto a resting ball at {speed} uu/s",
        "settle_ticks": 0,
        "car": {
            "location": [offset, 0, z],
            "rotation": [0, 0, 0],
            "velocity": [0, 0, -speed],
            "angular_velocity": [0, 0, 0],
            "boost": 0,
        },
        "ball": {"location": [0, 0, BALL_Z], "velocity": [0, 0, 0]},
        "steps": [{"ticks": TICKS}],
    }


def main() -> None:
    names = []
    for offset in OFFSETS:
        for speed in SPEEDS:
            name = f"wob_{offset}_{speed}"
            (OUT / f"{name}.json").write_text(json.dumps(tape(offset, speed), indent=2) + "\n", encoding="utf-8", newline="\n")
            names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
