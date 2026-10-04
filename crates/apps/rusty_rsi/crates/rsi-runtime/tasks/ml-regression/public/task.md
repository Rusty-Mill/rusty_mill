# ml-regression

Predict a numeric target `y` from three features.

- `data/train.csv` has a header `x1,x2,x3,y` and 200 training rows.
- `data/inputs.csv` has a header `x1,x2,x3` and one row per prediction.
- The target is a smooth, partly nonlinear function of the features plus noise.

Write one prediction (a number) per input row. The score is R² against the
true targets, clamped to [0, 1]; predicting the training mean scores about 0.

## Solution contract

Write one Python 3 file, standard library only. It runs as
`python3 solution.py data/inputs.csv output.txt` in a fresh directory,
with no network, a 10-second CPU limit and 512 MB of memory.
`RSI_SEED` in the environment holds a seed for any randomness.
Write exactly one answer per input line to `output.txt`.
