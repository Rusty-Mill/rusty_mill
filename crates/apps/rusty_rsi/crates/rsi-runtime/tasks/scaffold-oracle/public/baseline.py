"""Baseline: ask the oracle once per question and trust it."""
import sys

sys.path.insert(0, "data")
import oracle  # noqa: E402

inputs, output = sys.argv[1], sys.argv[2]
with open(inputs) as f, open(output, "w") as out:
    for question in f:
        out.write(oracle.ask(question.strip()) + "\n")
