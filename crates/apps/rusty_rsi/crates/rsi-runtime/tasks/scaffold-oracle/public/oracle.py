"""A deterministic stand-in for a weak model (the harness family's "LLM").

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
    match = re.fullmatch(r"What is (-?\d+) (plus|minus|times) (-?\d+)\?", question.strip())
    if not match:
        return "I don't know"
    a, op, b = int(match[1]), match[2], int(match[3])
    truth = _OPS[op](a, b)
    digest = hashlib.sha256(f"{question}|{attempt}".encode()).digest()
    if digest[0] < 170:
        return str(truth)
    offset = 1 + digest[1] % 3
    return str(truth + offset if digest[2] % 2 else truth - offset)
