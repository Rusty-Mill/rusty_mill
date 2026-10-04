# scaffold-oracle

Answer arithmetic questions using a noisy oracle.

- Each line of `data/questions.txt` is a question such as `What is 12 plus 7?`.
- `data/oracle.py` provides `ask(question, attempt=0) -> str`. About a third
  of its answers are slightly wrong; different `attempt` numbers can give
  different answers. Each question may be asked at most 5 times.
  Import it with `sys.path.insert(0, "data"); import oracle`.

Write one integer answer per question. The score is accuracy; asking once
and trusting the oracle scores about 0.8.

## Solution contract

Write one Python 3 file, standard library only. It runs as
`python3 solution.py data/questions.txt output.txt` in a fresh directory,
with no network, a 10-second CPU limit and 512 MB of memory.
`RSI_SEED` in the environment holds a seed for any randomness.
Write exactly one answer per input line to `output.txt`.
