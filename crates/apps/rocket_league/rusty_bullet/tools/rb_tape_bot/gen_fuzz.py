"""Seeded random controller tapes, for a holistic fidelity check.

    python -I tools/rb_tape_bot/gen_fuzz.py [count [first_seed]]

Writes tools/rb_tape_bot/experiments/fuzz_<seed>.json: a car on the floor near
the middle of the field, 6 s of random driving, boosting, handbrake turns,
jumps, dodges and air control, the ball out of the way. Run with
`run_batch.ps1 -Scenarios fuzz_1,fuzz_2,...` and score against the port with
`rb-verify --scenario ... --against ... --recorded-inputs`: what is left is
whatever the targeted probes did not think to try.
"""
import json
import pathlib
import random
import sys

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
TICKS = 720  # 6 s


def step(ticks, **inp):
    return dict({"ticks": ticks}, **inp)


def random_tape(rng: random.Random) -> list:
    steps = []
    spent = 0
    in_air_until = 0
    while spent < TICKS:
        length = rng.randint(12, 48)
        length = min(length, TICKS - spent)
        inp = {"throttle": rng.choice([1, 1, 1, 0, -1])}
        if rng.random() < 0.6:
            inp["steer"] = rng.choice([-1, -0.5, 0.5, 1, 0, 0])
        if rng.random() < 0.45:
            inp["boost"] = True
        if rng.random() < 0.12:
            inp["handbrake"] = True
        airborne = spent < in_air_until
        if not airborne and rng.random() < 0.18:
            inp["jump"] = True
            in_air_until = spent + rng.randint(30, 90)
            length = min(length, rng.randint(2, 20))
        elif airborne and rng.random() < 0.3:
            inp["jump"] = True  # a second press: a double jump or a dodge
            length = min(length, 2)
            for axis in ("pitch", "yaw", "roll"):
                if rng.random() < 0.6:
                    inp[axis] = rng.choice([-1, 1])
        elif airborne:
            for axis in ("pitch", "yaw", "roll"):
                if rng.random() < 0.5:
                    inp[axis] = rng.choice([-1, -0.5, 0.5, 1])
        steps.append(step(length, **inp))
        spent += length
    return steps


def main() -> None:
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 24
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    names = []
    for seed in range(first, first + count):
        rng = random.Random(seed)
        x, y = rng.randint(-2000, 2000), rng.randint(-3000, 3000)
        yaw = rng.uniform(-3.14, 3.14)
        d = {
            "name": f"fuzz_{seed}: seeded random tape, 6 s",
            "settle_ticks": 60,
            "car": {"location": [x, y, 17], "rotation": [-0.0096, yaw, 0], "velocity": [0, 0, 0],
                    "angular_velocity": [0, 0, 0], "boost": 100},
            "ball": {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]},
            "steps": random_tape(rng),
        }
        (OUT / f"fuzz_{seed}.json").write_text(json.dumps(d, indent=2) + "\n", encoding="utf-8", newline="\n")
        names.append(f"fuzz_{seed}")
    print(",".join(names))


if __name__ == "__main__":
    main()
