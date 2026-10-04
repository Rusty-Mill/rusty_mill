"""Baseline: predict the training mean of y for every row."""
import csv
import sys

inputs, output = sys.argv[1], sys.argv[2]
with open("data/train.csv") as f:
    ys = [float(row["y"]) for row in csv.DictReader(f)]
mean = sum(ys) / len(ys)
with open(inputs) as f:
    rows = list(csv.DictReader(f))
with open(output, "w") as f:
    f.writelines(f"{mean}\n" for _ in rows)
