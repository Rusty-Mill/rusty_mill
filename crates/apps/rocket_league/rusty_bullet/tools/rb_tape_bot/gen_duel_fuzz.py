"""Seeded random two-car drives that start at each other, for car-car contacts.

    python -I tools/rb_tape_bot/gen_duel_fuzz.py [count [first_seed]]

Writes tools/rb_tape_bot/experiments/duel_<seed>.json: two cars 450 to 800 uu apart on a
random line through the middle of the field, each facing the other (within 8 degrees, the second car up to 110 uu off the line), a boosted run at each other (40 to 80 ticks) and then the random tape
of `gen_fuzz.py` for each, 6 s in all. The cars are enemies in half the seeds, so supersonic
hits can demolish; the ball is out of the way. Score with `RB_OTHERS=1 rb-verify --scenario
... --against ... --recorded-inputs`: car 0 and the second car's error lines.
"""
import json
import math
import pathlib
import random
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from gen_fuzz import TICKS, random_tape, step  # noqa: E402

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
RUN_TICKS_BOTH = (40, 75)  # about when two boosting cars 450 to 800 uu apart meet
RUN_TICKS_ALONE = (80, 130)  # one boosting car covering the whole distance


def car_state(x: float, y: float, yaw: float, boost: int) -> dict:
    return {
        "location": [round(x), round(y), 17],
        "rotation": [0, round(yaw, 4), 0],
        "velocity": [0, 0, 0],
        "angular_velocity": [0, 0, 0],
        "boost": boost,
    }


def scenario(seed: int) -> dict:
    rng = random.Random(seed)
    angle = rng.uniform(0, math.tau)
    distance = rng.uniform(450, 800)
    cx, cy = rng.uniform(-800, 800), rng.uniform(-800, 800)
    dx, dy = math.cos(angle) * distance / 2, math.sin(angle) * distance / 2
    boost = rng.choice([33, 100])
    # Each car faces the other, within 8 degrees.
    face_a = angle + math.pi + math.radians(rng.uniform(-8, 8))
    face_b = angle + math.radians(rng.uniform(-8, 8))
    # A sideways offset of the second car, so the hits are off centre.
    side = rng.uniform(-110, 110)
    ox, oy = -math.sin(angle) * side, math.cos(angle) * side
    team = rng.choice([0, 1])
    # Same team: the hivemind plays both tapes and the cars meet in the middle. Enemy team: the
    # second car stands still, so the first has to cover the whole distance.
    run_ticks = rng.randint(*(RUN_TICKS_BOTH if team == 0 else RUN_TICKS_ALONE))
    run_a = [step(run_ticks, throttle=1, boost=True)]
    run_b = [step(run_ticks, throttle=1, boost=True)] if team == 0 else [step(run_ticks)]
    rest = TICKS - run_ticks
    tail_a = [s for s in random_tape(rng) if s]
    tail_b = [s for s in random_tape(rng) if s] if team == 0 else [step(TICKS)]
    return {
        "name": f"duel_{seed}: two cars run at each other, then seeded random tapes",
        "settle_ticks": 0,
        "car": car_state(cx + dx, cy + dy, face_a, boost),
        "ball": {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": run_a + trim(tail_a, rest),
        "others": [
            {
                "car": car_state(cx - dx + ox, cy - dy + oy, face_b, boost),
                "steps": run_b + trim(tail_b, rest),
                "team": team,
            }
        ],
    }


def trim(steps: list, ticks: int) -> list:
    """The first `ticks` ticks of `steps`."""
    out, spent = [], 0
    for s in steps:
        if spent >= ticks:
            break
        length = min(s["ticks"], ticks - spent)
        out.append({**s, "ticks": length})
        spent += length
    return out


def main() -> None:
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 24
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 801
    names = []
    for seed in range(first, first + count):
        name = f"duel_{seed}"
        (OUT / f"{name}.json").write_text(json.dumps(scenario(seed), indent=2) + chr(10), encoding="utf-8", newline=chr(10))
        names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
