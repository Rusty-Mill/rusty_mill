"""Process-wide metrics + per-stage timing — stdlib only (no prometheus_client).

A tiny thread-safe registry of **counters** and **histograms**, rendered in
Prometheus text exposition format at `GET /metrics`. Consistent with the rest of
the service (no heavy deps) and testable everywhere; point a real client at
`render()` if a deployment wants one.

Instruments three things: upload outcomes (`UPLOADS`), scoring results
(`SCORED`), per-stage scoring duration (`SCORING_STAGE`), and HTTP RED metrics
(`HTTP_REQUESTS`, `HTTP_LATENCY`). `Histogram.time()` also emits a structured log
line per stage.
"""

from __future__ import annotations

import logging
import threading
import time
from contextlib import contextmanager

logger = logging.getLogger("rls.metrics")

# Histogram buckets (seconds), tuned for subprocess scoring + DB writes + HTTP.
_BUCKETS = (0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0)
_lock = threading.Lock()
_REGISTRY: list = []


def _esc(v: str) -> str:
    return v.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")


def _fnum(x: float) -> str:
    return "%g" % x


def _labels(pairs: list[tuple[str, str]]) -> str:
    if not pairs:
        return ""
    return "{" + ",".join(f'{k}="{_esc(v)}"' for k, v in pairs) + "}"


class Counter:
    type = "counter"

    def __init__(self, name: str, help_: str) -> None:
        self.name, self.help = name, help_
        self._v: dict[tuple, float] = {}
        _REGISTRY.append(self)

    def inc(self, amount: float = 1.0, **labels: str) -> None:
        key = tuple(sorted(labels.items()))
        with _lock:
            self._v[key] = self._v.get(key, 0.0) + amount

    def _reset(self) -> None:
        self._v.clear()

    def _render(self) -> list[str]:
        with _lock:
            items = sorted(self._v.items())
        return [f"{self.name}{_labels(list(k))} {_fnum(v)}" for k, v in items]


class Histogram:
    type = "histogram"

    def __init__(self, name: str, help_: str, buckets: tuple = _BUCKETS) -> None:
        self.name, self.help, self.buckets = name, help_, buckets
        self._counts: dict[tuple, list[int]] = {}
        self._sum: dict[tuple, float] = {}
        _REGISTRY.append(self)

    def observe(self, value: float, **labels: str) -> None:
        key = tuple(sorted(labels.items()))
        idx = len(self.buckets)
        for i, b in enumerate(self.buckets):
            if value <= b:
                idx = i
                break
        with _lock:
            counts = self._counts.get(key)
            if counts is None:
                counts = [0] * (len(self.buckets) + 1)
                self._counts[key] = counts
            counts[idx] += 1
            self._sum[key] = self._sum.get(key, 0.0) + value

    @contextmanager
    def time(self, **labels: str):
        start = time.perf_counter()
        try:
            yield
        finally:
            dt = time.perf_counter() - start
            self.observe(dt, **labels)
            logger.info("stage=%s duration_ms=%.1f", labels, dt * 1000)

    def _reset(self) -> None:
        self._counts.clear()
        self._sum.clear()

    def _render(self) -> list[str]:
        with _lock:
            items = [(k, list(c), self._sum[k]) for k, c in sorted(self._counts.items())]
        lines: list[str] = []
        for key, counts, total in items:
            base = list(key)
            cum = 0
            for i, b in enumerate(self.buckets):
                cum += counts[i]
                lines.append(f"{self.name}_bucket{_labels(base + [('le', _fnum(b))])} {cum}")
            cum += counts[-1]
            lines.append(f"{self.name}_bucket{_labels(base + [('le', '+Inf')])} {cum}")
            lines.append(f"{self.name}_sum{_labels(base)} {_fnum(total)}")
            lines.append(f"{self.name}_count{_labels(base)} {cum}")
        return lines


def reset() -> None:
    """Clear all series (tests)."""
    for m in _REGISTRY:
        m._reset()


def render() -> str:
    """Prometheus text exposition for every registered metric."""
    out: list[str] = []
    for m in _REGISTRY:
        body = m._render()
        if not body:
            continue
        out.append(f"# HELP {m.name} {m.help}")
        out.append(f"# TYPE {m.name} {m.type}")
        out.extend(body)
    return "\n".join(out) + "\n"


# --- the metrics the service records ---
UPLOADS = Counter("rls_uploads_total", "Replay uploads by outcome.")
SCORED = Counter("rls_scored_total", "Scoring results by outcome.")
SCORING_STAGE = Histogram("rls_scoring_stage_seconds", "Scoring pipeline stage duration (s).")
HTTP_REQUESTS = Counter("rls_http_requests_total", "HTTP requests by route and status.")
HTTP_LATENCY = Histogram("rls_http_request_seconds", "HTTP request latency (s).")
