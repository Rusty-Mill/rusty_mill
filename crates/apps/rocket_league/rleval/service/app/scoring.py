"""The scoring port and its adapters (ports-and-adapters, spec §3/§6).

The service treats *scoring* as an external capability behind the [`Scorer`]
protocol, so the HTTP/persistence layer is testable without the Rust toolchain or
real `.replay` files. [`SubprocessScorer`] is the production adapter — it shells
out to the Rust `replay-scoring` worker, which is the source of truth for the
rubric. Tests inject a fake.
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
from dataclasses import dataclass
from typing import Protocol


class ScoringError(RuntimeError):
    """The worker failed or produced no usable report."""


@dataclass
class PlayerScore:
    """One player's decision-discipline report (maps to a `Report` row)."""

    player_id: str
    composite: float
    first_man: float
    second_man: float
    general: float
    licence: str
    player_type: str
    main_leak: str
    focus_chapter: str
    confidence: str  # "ok" | "low_confidence"
    metrics_json: str  # the per-metric breakdown, serialized


@dataclass
class ScoreResult:
    parser_version: str
    score_config_version: str
    players: list[PlayerScore]
    # The full-lobby report HTML (from `--html`), for PDF rendering (§4.10). May
    # be absent if the worker didn't emit it.
    report_html: str | None = None
    # The serialized canonical match (from `--dump-canonical`), for a no-re-parse
    # re-score (§9). Present only when canonical caching is enabled.
    canonical_blob: bytes | None = None


class Scorer(Protocol):
    def score(self, blob: bytes, replay_id: str) -> ScoreResult: ...

    def score_from_canonical(
        self, canonical_blob: bytes, replay_id: str
    ) -> ScoreResult: ...

    def render_player_html(
        self, canonical_blob: bytes, replay_id: str, player_id: str
    ) -> str: ...


def _player_from_report(r: dict) -> PlayerScore:
    """Map one Rust `Report` JSON object to a [`PlayerScore`]."""
    return PlayerScore(
        player_id=r["target_player"],
        composite=r["composite"],
        first_man=r["first_man"],
        second_man=r["second_man"],
        general=r["general"],
        licence=r["licence"],
        player_type=r["player_type"],
        main_leak=r["main_leak"],
        focus_chapter=r["focus_chapter"],
        confidence=r["confidence"],
        metrics_json=json.dumps(r.get("metrics", [])),
    )


class SubprocessScorer:
    """Runs the Rust `replay-scoring` worker on a blob and parses its JSON.

    Without `--player` the worker emits a JSON *array* of per-player reports (the
    full lobby); with one player it emits a single object. We handle both.
    """

    def __init__(
        self, worker_bin: str, timeout_s: int = 300, cache_canonical: bool = False
    ) -> None:
        self.worker_bin = worker_bin
        self.timeout_s = timeout_s
        self.cache_canonical = cache_canonical

    def score(self, blob: bytes, replay_id: str) -> ScoreResult:
        """Parse + score a raw replay (optionally dumping the canonical blob)."""
        with tempfile.TemporaryDirectory() as d:
            in_path = os.path.join(d, f"{replay_id}.replay")
            with open(in_path, "wb") as f:
                f.write(blob)
            can_out = os.path.join(d, "canonical.json") if self.cache_canonical else None
            input_args = [in_path]
            if can_out:
                input_args += ["--dump-canonical", can_out]
            return self._run(d, input_args, can_out)

    def score_from_canonical(self, canonical_blob: bytes, replay_id: str) -> ScoreResult:
        """Score from a cached canonical match — the identical core, no re-parse."""
        with tempfile.TemporaryDirectory() as d:
            can_in = os.path.join(d, f"{replay_id}.canonical.json")
            with open(can_in, "wb") as f:
                f.write(canonical_blob)
            return self._run(d, ["--from-canonical", can_in], None)

    def render_player_html(
        self, canonical_blob: bytes, replay_id: str, player_id: str
    ) -> str:
        """Render one player's scoped report HTML from the cached canonical match
        (`--from-canonical --player`, the same core, no re-parse) — for per-player
        PDF export (§4.10)."""
        with tempfile.TemporaryDirectory() as d:
            can_in = os.path.join(d, f"{replay_id}.canonical.json")
            with open(can_in, "wb") as f:
                f.write(canonical_blob)
            html_path = os.path.join(d, "player.html")
            try:
                proc = subprocess.run(
                    [
                        self.worker_bin,
                        "--from-canonical",
                        can_in,
                        "--player",
                        player_id,
                        "--html",
                        html_path,
                    ],
                    capture_output=True,
                    text=True,
                    timeout=self.timeout_s,
                )
            except (OSError, subprocess.TimeoutExpired) as e:
                raise ScoringError(f"worker did not run: {e}") from e
            if proc.returncode != 0:
                raise ScoringError(
                    f"worker exit {proc.returncode}: {proc.stderr[-500:].strip()}"
                )
            if not os.path.exists(html_path):
                raise ScoringError("worker produced no player HTML")
            with open(html_path, encoding="utf-8") as f:
                return f.read()

    def _run(
        self, d: str, input_args: list[str], canonical_out: str | None
    ) -> ScoreResult:
        out_path = os.path.join(d, "report.json")
        html_path = os.path.join(d, "report.html")
        try:
            # One invocation emits the per-player JSON + the full-lobby report HTML
            # (the latter feeds PDF rendering).
            proc = subprocess.run(
                [self.worker_bin, *input_args, "--json", out_path, "--html", html_path],
                capture_output=True,
                text=True,
                timeout=self.timeout_s,
            )
        except (OSError, subprocess.TimeoutExpired) as e:
            raise ScoringError(f"worker did not run: {e}") from e
        if proc.returncode != 0:
            raise ScoringError(
                f"worker exit {proc.returncode}: {proc.stderr[-500:].strip()}"
            )
        with open(out_path) as f:
            data = json.load(f)
        report_html = None
        if os.path.exists(html_path):
            with open(html_path, encoding="utf-8") as f:
                report_html = f.read()
        canonical_blob = None
        if canonical_out and os.path.exists(canonical_out):
            with open(canonical_out, "rb") as f:
                canonical_blob = f.read()

        reports = data if isinstance(data, list) else [data]
        if not reports:
            raise ScoringError("worker produced no reports")
        first = reports[0]
        return ScoreResult(
            parser_version=first["parser_version"],
            score_config_version=first["score_config_version"],
            players=[_player_from_report(r) for r in reports],
            report_html=report_html,
            canonical_blob=canonical_blob,
        )
