"""Trims a tape-bot capture to its scenario's tape, for use as a test fixture.

    python -I tools/rb_tape_bot/make_fixture.py <scenario-name> <capture.jsonl>

Writes tools/rb_tape_bot/fixtures/<scenario-name>.jsonl: the frames from the
state-set frame (the first whose car is within 5 uu of the scenario's start)
to three ticks past the end of the tape. The scenario is looked up in
`scenarios/` then `experiments/`. A capture holds no personal data (the car
and ball of a scripted bot run), unlike a replay of a real match.
"""
import json
import pathlib
import sys

BOT = pathlib.Path(__file__).resolve().parent
START_RADIUS = 5.0
TICK = 1.0 / 120.0


def scenario_path(name: str) -> pathlib.Path:
    for folder in ("scenarios", "experiments"):
        path = BOT / folder / f"{name}.json"
        if path.exists():
            return path
    raise SystemExit(f"no scenario named {name}")


def total_ticks(scenario: dict) -> int:
    return scenario.get("settle_ticks", 0) + sum(step["ticks"] for step in scenario["steps"])


def main() -> None:
    name, capture = sys.argv[1], pathlib.Path(sys.argv[2])
    scenario = json.loads(scenario_path(name).read_text(encoding="utf-8"))
    start = scenario["car"]["location"]
    lines = capture.read_text(encoding="utf-8").splitlines()
    rows = [json.loads(line) for line in lines]

    def near(row: dict) -> bool:
        p = row["cars"][0]["position"]
        d2 = sum((p[a] - v) ** 2 for a, v in zip("xyz", start))
        return d2 ** 0.5 <= START_RADIUS

    first = next((i for i, row in enumerate(rows) if near(row)), None)
    if first is None:
        raise SystemExit(f"{capture}: no frame has the car within {START_RADIUS} uu of the start")
    end_secs = rows[first]["timestamp_secs"] + (total_ticks(scenario) + 3) * TICK
    kept = [line for line, row in zip(lines[first:], rows[first:]) if row["timestamp_secs"] <= end_secs]
    out = BOT / "fixtures" / f"{name}.jsonl"
    out.parent.mkdir(exist_ok=True)
    out.write_text("\n".join(kept) + "\n", encoding="utf-8", newline="\n")
    print(f"{out.relative_to(BOT.parent.parent)}: {len(kept)} frames, {out.stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
