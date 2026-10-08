"""Three-car scenarios (PARITY-PLAN workstream D, RB-RESEARCH-O018).

    python -I tools/rb_tape_bot/gen_tri.py

Writes experiments/tri_<name>.json: three cars, so a bump can pass on to a third car or
two cars can reach one (the port's contact solve shares one budget per body). Prints the
names. Cars face +y unless noted; `team` makes cars enemies.
"""
import json
import math
import pathlib

OUT = pathlib.Path(__file__).resolve().parent / "experiments"
TICKS = 300


def car(x: float, y: float, vy: float, yaw: float = 90, boost: int = 0) -> dict:
    return {
        "location": [x, y, 17],
        "rotation": [-0.0096, math.radians(yaw), 0],
        "velocity": [0, vy, 0],
        "angular_velocity": [0, 0, 0],
        "boost": boost,
    }


def other(c: dict, team: int, steps: list) -> dict:
    return {"car": c, "steps": steps, "team": team}


def tape(name: str, text: str, attacker: dict, attacker_steps: list, others: list) -> dict:
    return {
        "name": f"tri_{name}: {text}",
        "settle_ticks": 0,
        "car": attacker,
        "ball": {"location": [3500, 3500, 93.15], "velocity": [0, 0, 0]},
        "steps": attacker_steps,
        "others": others,
    }


def drive(ticks: int = TICKS) -> list:
    return [{"ticks": ticks, "throttle": 1}]


def boost_then_coast() -> list:
    return [{"ticks": 150, "throttle": 1, "boost": True}, {"ticks": TICKS - 150}]


def idle() -> list:
    return [{"ticks": TICKS}]


def tapes() -> dict:
    return {
        "chain": tape(
            "chain", "A at 1400 uu/s bumps teammate B, which is 160 uu short of teammate C",
            car(0, -700, 1400), drive(),
            [other(car(0, 0, 0), 0, idle()), other(car(0, 230, 0), 0, idle())],
        ),
        "chain_enemy": tape(
            "chain_enemy", "A at 1400 uu/s bumps enemy B, which is 160 uu short of enemy C",
            car(0, -700, 1400), drive(),
            [other(car(0, 0, 0), 1, idle()), other(car(0, 230, 0), 1, idle())],
        ),
        "pinch": tape(
            "pinch", "A and B at 1400 uu/s from both sides into stopped enemy C",
            car(0, -900, 1400), drive(),
            [other(car(0, 900, -1400, -90), 0, drive()), other(car(0, 0, 0), 1, idle())],
        ),
        "double_demo": tape(
            "double_demo", "A boosts at 2300 uu/s through enemies B and C, 500 uu apart",
            car(0, -808, 2300, boost=100), boost_then_coast(),
            [other(car(0, 0, 0), 1, idle()), other(car(0, 500, 0), 1, idle())],
        ),
        "two_on_one": tape(
            "two_on_one", "A and B side by side at 2300 uu/s into stopped enemy C",
            car(-70, -808, 2300, boost=100), boost_then_coast(),
            [other(car(70, -808, 2300, boost=100), 0, boost_then_coast()), other(car(0, 0, 0), 1, idle())],
        ),
        "mate_into_enemy": tape(
            "mate_into_enemy", "A at 2300 uu/s bumps teammate B (not a demolition) which runs on into stopped enemy C",
            car(0, -808, 2300, boost=100), boost_then_coast(),
            [other(car(0, 0, 0), 0, idle()), other(car(0, 450, 0), 1, idle())],
        ),
    }


def main() -> None:
    names = []
    for key, scenario in tapes().items():
        name = f"tri_{key}"
        text = json.dumps(scenario, indent=2) + chr(10)
        (OUT / f"{name}.json").write_text(text, encoding="utf-8", newline=chr(10))
        names.append(name)
    print(",".join(names))


if __name__ == "__main__":
    main()
