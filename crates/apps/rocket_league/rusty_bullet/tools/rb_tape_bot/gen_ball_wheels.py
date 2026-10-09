"""Wheels-first drops onto a resting ball (PARITY-PLAN workstream B step 2, RB-RESEARCH-O014).

    python -I tools/rb_tape_bot/gen_ball_wheels.py [--held-out]

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
# Held out from the fits: other offsets and speeds, and a drop beside the nose axis
# (`wobh_<offset>_<side>_<speed>`, `side` uu to the car's right).
HELD_OUT = [(25, 0, 350), (75, 0, 350), (150, 0, 350), (25, 0, 750), (75, 0, 750), (150, 0, 750), (50, 40, 500), (100, 40, 500)]
TICKS = 180


def tape(offset: int, speed: int, side: int = 0, prefix: str = "wob") -> dict:
    # Car origin 40 uu above the ball's top: the wheels (about 33 uu below the origin)
    # reach the ball within a tick or two.
    z = BALL_Z + BALL_RADIUS + 40
    return {
        "name": f"{prefix}_{offset}_{speed}: level car {offset} uu off centre{f' and {side} uu to the side' if side else ''} falls onto a resting ball at {speed} uu/s",
        "settle_ticks": 0,
        "car": {
            "location": [offset, side, z],
            "rotation": [0, 0, 0],
            "velocity": [0, 0, -speed],
            "angular_velocity": [0, 0, 0],
            "boost": 0,
        },
        "ball": {"location": [0, 0, BALL_Z], "velocity": [0, 0, 0]},
        "steps": [{"ticks": TICKS}],
    }


def main() -> None:
    import sys

    if "--held-out" in sys.argv:
        names = []
        for offset, side, speed in HELD_OUT:
            name = f"wobh_{offset}_{side}_{speed}"
            text = json.dumps(tape(offset, speed, side, "wobh"), indent=2) + chr(10)
            (OUT / f"{name}.json").write_text(text, encoding="utf-8", newline=chr(10))
            names.append(name)
        print(",".join(names))
        return
    names = []
    for offset in OFFSETS:
        for speed in SPEEDS:
            name = f"wob_{offset}_{speed}"
            (OUT / f"{name}.json").write_text(json.dumps(tape(offset, speed), indent=2) + "\n", encoding="utf-8", newline="\n")
            names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
