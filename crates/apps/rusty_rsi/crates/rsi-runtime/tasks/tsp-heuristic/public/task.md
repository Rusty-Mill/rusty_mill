# tsp-heuristic

Find short round trips through cities in the unit square.

- Each line of `data/instances.txt` is one instance: `x0 y0 x1 y1 ...`,
  between 25 and 50 cities.

For each instance write a permutation of the city indices `0..n-1`,
space-separated, on its own line. The tour returns to its start. The score
is the mean over instances of min(1, reference length / your length); the
order as given scores about 0.2.

## Solution contract

Write one Python 3 file, standard library only. It runs as
`python3 solution.py data/instances.txt output.txt` in a fresh directory,
with no network, a 10-second CPU limit and 512 MB of memory.
`RSI_SEED` in the environment holds a seed for any randomness.
Write exactly one answer per input line to `output.txt`.
