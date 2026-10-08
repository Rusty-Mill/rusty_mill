"""Scores every capture in a batch directory with rb-verify, one line each.

    python -I tools/rb_tape_bot/score_dir.py replays/batch_<stamp> [name-prefix]

A quick look at car and ball error per scenario (the full table is
`run_batch.ps1 -ScoreOnly`). Scenario files are looked up in `scenarios/`
then `experiments/`.
"""
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
VERIFY = ROOT / "target" / "release" / "rb-verify.exe"
BOT = ROOT / "tools" / "rb_tape_bot"


def scenario_path(name: str) -> pathlib.Path | None:
    for folder in ("scenarios", "experiments"):
        path = BOT / folder / f"{name}.json"
        if path.exists():
            return path
    return None


def score(scenario: pathlib.Path, capture: pathlib.Path) -> str:
    out = subprocess.run(
        [str(VERIFY), "--scenario", str(scenario), "--against", str(capture)],
        capture_output=True,
        text=True,
    )
    text = out.stdout + out.stderr
    lag = re.search(r"lag (\d+) ticks", text)
    car = re.search(r"position error: mean ([\d.]+) uu, max ([\d.]+)", text)
    ball = re.search(r"ball error: mean ([\d.]+) uu, max ([\d.]+)", text)
    if not (lag and car and ball):
        return "unscored: " + text.strip().splitlines()[0][:100]
    return (
        f"lag {lag.group(1)}  car {car.group(1):>6} / {car.group(2):>6}"
        f"  ball {ball.group(1):>6} / {ball.group(2):>6}"
    )


def main() -> None:
    batch = pathlib.Path(sys.argv[1])
    prefix = sys.argv[2] if len(sys.argv) > 2 else ""
    for capture in sorted(batch.glob("*_run1.jsonl")):
        name = capture.name[: -len("_run1.jsonl")]
        if not name.startswith(prefix):
            continue
        scenario = scenario_path(name)
        print(f"{name:<26}", score(scenario, capture) if scenario else "no scenario file")


if __name__ == "__main__":
    main()
