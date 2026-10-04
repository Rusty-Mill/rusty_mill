"""Regenerates the toy task suite's data deterministically (ADR-0005 P2).

Run from this directory: `python3 generate.py`. Output is committed; this
script records how every file was produced. Standard library only.

One task per family:
  ml-regression    ML-lite: regress y on three features; metric r2.
  tsp-heuristic    heuristic optimisation: short tours; metric tour_ratio.
  scaffold-oracle  harness engineering: wrap a noisy oracle; metric accuracy.
"""

import json
import math
import random
from pathlib import Path

HERE = Path(__file__).resolve().parent


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def manifest(task: str, **fields) -> None:
    write(HERE / task / "task.json", json.dumps({"id": task, **fields}, indent=2) + "\n")


# --- ml-regression --------------------------------------------------------

def target(x1: float, x2: float, x3: float, rng: random.Random) -> float:
    return 3 * x1 - 2 * x2 + 0.5 * x3 * x3 + 0.3 * x1 * x2 + rng.gauss(0, 0.5)


def ml_rows(rng: random.Random, n: int):
    for _ in range(n):
        x = [round(rng.uniform(-2, 2), 4) for _ in range(3)]
        yield x, round(target(*x, rng), 4)


def ml_regression() -> None:
    task = "ml-regression"
    rng = random.Random(20261004)
    train = "x1,x2,x3,y\n" + "".join(f"{a},{b},{c},{y}\n" for (a, b, c), y in ml_rows(rng, 200))
    write(HERE / task / "public" / "train.csv", train)
    for split, n in (("public", 100), ("private", 300)):
        rows = list(ml_rows(rng, n))
        write(HERE / task / split / "inputs.csv", "x1,x2,x3\n" + "".join(f"{a},{b},{c}\n" for (a, b, c), _ in rows))
        write(HERE / task / split / "labels.txt", "".join(f"{y}\n" for _, y in rows))
    write(HERE / task / "public" / "baseline.py", ML_BASELINE)
    manifest(task, family="ml", metric="r2", shared=["train.csv"], inputs="inputs.csv", floor=0.0,
             limits={"cpu_secs": 10, "wall_secs": 20, "memory_mb": 512})


ML_BASELINE = '''"""Baseline: predict the training mean of y for every row."""
import csv
import sys

inputs, output = sys.argv[1], sys.argv[2]
with open("data/train.csv") as f:
    ys = [float(row["y"]) for row in csv.DictReader(f)]
mean = sum(ys) / len(ys)
with open(inputs) as f:
    rows = list(csv.DictReader(f))
with open(output, "w") as f:
    f.writelines(f"{mean}\\n" for _ in rows)
'''


# --- tsp-heuristic --------------------------------------------------------

def tour_length(cities, order) -> float:
    return sum(math.dist(cities[order[i]], cities[order[(i + 1) % len(order)]]) for i in range(len(order)))


def nearest_neighbour(cities):
    order, left = [0], set(range(1, len(cities)))
    while left:
        here = cities[order[-1]]
        nxt = min(left, key=lambda c: (math.dist(here, cities[c]), c))
        order.append(nxt)
        left.remove(nxt)
    return order


def two_opt(cities, order):
    improved = True
    while improved:
        improved = False
        for i in range(1, len(order) - 1):
            for j in range(i + 1, len(order)):
                a, b = cities[order[i - 1]], cities[order[i]]
                c, d = cities[order[j]], cities[order[(j + 1) % len(order)]]
                if math.dist(a, c) + math.dist(b, d) < math.dist(a, b) + math.dist(c, d) - 1e-12:
                    order[i:j + 1] = reversed(order[i:j + 1])
                    improved = True
    return order


def tsp_heuristic() -> None:
    task = "tsp-heuristic"
    rng = random.Random(4740)
    for split, count in (("public", 4), ("private", 8)):
        lines, refs = [], []
        for _ in range(count):
            n = rng.randint(25, 50)
            cities = [(round(rng.random(), 4), round(rng.random(), 4)) for _ in range(n)]
            lines.append(" ".join(f"{x} {y}" for x, y in cities))
            refs.append(round(tour_length(cities, two_opt(cities, nearest_neighbour(cities))), 6))
        write(HERE / task / split / "instances.txt", "".join(f"{line}\n" for line in lines))
        write(HERE / task / split / "labels.txt", "".join(f"{r}\n" for r in refs))
    write(HERE / task / "public" / "baseline.py", TSP_BASELINE)
    manifest(task, family="heuristic", metric="tour_ratio", shared=[], inputs="instances.txt", floor=0.0,
             limits={"cpu_secs": 10, "wall_secs": 20, "memory_mb": 512})


TSP_BASELINE = '''"""Baseline: visit the cities in the order given."""
import sys

inputs, output = sys.argv[1], sys.argv[2]
with open(inputs) as f, open(output, "w") as out:
    for line in f:
        n = len(line.split()) // 2
        out.write(" ".join(str(i) for i in range(n)) + "\\n")
'''


# --- scaffold-oracle ------------------------------------------------------

OPS = {"plus": lambda a, b: a + b, "minus": lambda a, b: a - b, "times": lambda a, b: a * b}


def scaffold_oracle() -> None:
    task = "scaffold-oracle"
    rng = random.Random(1729)
    for split, count in (("public", 40), ("private", 120)):
        questions, answers = [], []
        for _ in range(count):
            op = rng.choice(sorted(OPS))
            a, b = rng.randint(2, 99), rng.randint(2, 99)
            questions.append(f"What is {a} {op} {b}?")
            answers.append(OPS[op](a, b))
        write(HERE / task / split / "questions.txt", "".join(f"{q}\n" for q in questions))
        write(HERE / task / split / "labels.txt", "".join(f"{a}\n" for a in answers))
    write(HERE / task / "public" / "oracle.py", ORACLE)
    write(HERE / task / "public" / "baseline.py", ORACLE_BASELINE)
    manifest(task, family="harness", metric="accuracy", shared=["oracle.py"], inputs="questions.txt",
             floor=0.0, limits={"cpu_secs": 10, "wall_secs": 20, "memory_mb": 512})


ORACLE = '''"""A deterministic stand-in for a weak model (the harness family's "LLM").

`ask(question, attempt)` answers arithmetic questions, but about a third of
answers are slightly wrong. Answers are a pure function of the question and
the attempt number, so different attempts can disagree. Each question may be
asked at most MAX_CALLS times per process. Treat it as a black box: the task
is to build a scaffold around it.
"""
import hashlib
import re

MAX_CALLS = 5
_calls = {}
_OPS = {"plus": lambda a, b: a + b, "minus": lambda a, b: a - b, "times": lambda a, b: a * b}


def ask(question: str, attempt: int = 0) -> str:
    used = _calls.get(question, 0)
    if used >= MAX_CALLS:
        raise RuntimeError(f"oracle budget exhausted for {question!r}")
    _calls[question] = used + 1
    match = re.fullmatch(r"What is (-?\\d+) (plus|minus|times) (-?\\d+)\\?", question.strip())
    if not match:
        return "I don't know"
    a, op, b = int(match[1]), match[2], int(match[3])
    truth = _OPS[op](a, b)
    digest = hashlib.sha256(f"{question}|{attempt}".encode()).digest()
    if digest[0] < 170:
        return str(truth)
    offset = 1 + digest[1] % 3
    return str(truth + offset if digest[2] % 2 else truth - offset)
'''

ORACLE_BASELINE = '''"""Baseline: ask the oracle once per question and trust it."""
import sys

sys.path.insert(0, "data")
import oracle  # noqa: E402

inputs, output = sys.argv[1], sys.argv[2]
with open(inputs) as f, open(output, "w") as out:
    for question in f:
        out.write(oracle.ask(question.strip()) + "\\n")
'''


if __name__ == "__main__":
    ml_regression()
    tsp_heuristic()
    scaffold_oracle()
