"""Baseline: visit the cities in the order given."""
import sys

inputs, output = sys.argv[1], sys.argv[2]
with open(inputs) as f, open(output, "w") as out:
    for line in f:
        n = len(line.split()) // 2
        out.write(" ".join(str(i) for i in range(n)) + "\n")
