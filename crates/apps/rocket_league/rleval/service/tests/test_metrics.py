"""Observability (§): the stdlib metrics registry and the `/metrics` endpoint."""

from __future__ import annotations

import io

from app import metrics

H = {"X-Account-Email": "u@example.com"}


def _file(content: bytes = b"obs-replay"):
    return {"file": ("r.replay", io.BytesIO(content), "application/octet-stream")}


def test_counter_renders_with_labels():
    metrics.reset()
    metrics.UPLOADS.inc(outcome="accepted")
    metrics.UPLOADS.inc(outcome="accepted")
    metrics.UPLOADS.inc(outcome="deduped")
    out = metrics.render()
    assert "# TYPE rls_uploads_total counter" in out
    assert 'rls_uploads_total{outcome="accepted"} 2' in out
    assert 'rls_uploads_total{outcome="deduped"} 1' in out


def test_histogram_buckets_are_cumulative_with_sum_and_count():
    metrics.reset()
    for v in (0.001, 0.2, 3.0):
        metrics.SCORING_STAGE.observe(v, stage="score")
    out = metrics.render()
    n = "rls_scoring_stage_seconds"
    assert "# TYPE rls_scoring_stage_seconds histogram" in out
    assert f'{n}_bucket{{stage="score",le="0.005"}} 1' in out  # only 0.001
    assert f'{n}_bucket{{stage="score",le="0.25"}} 2' in out   # +0.2
    assert f'{n}_bucket{{stage="score",le="+Inf"}} 3' in out
    assert f'{n}_count{{stage="score"}} 3' in out
    assert f'{n}_sum{{stage="score"}} 3.201' in out


def test_time_context_manager_records_one_observation():
    metrics.reset()
    with metrics.SCORING_STAGE.time(stage="persist"):
        pass
    assert 'rls_scoring_stage_seconds_count{stage="persist"} 1' in metrics.render()


def test_metrics_endpoint_reflects_upload_and_scoring(client, make_account):
    metrics.reset()
    make_account(credits_n=5)
    assert client.post("/v1/replays", files=_file(), headers=H).status_code == 202

    resp = client.get("/metrics")
    assert resp.status_code == 200
    assert "text/plain" in resp.headers["content-type"]
    body = resp.text
    # Upload outcome + (inline fake) scoring result + per-stage timing + HTTP RED.
    assert 'rls_uploads_total{outcome="accepted"} 1' in body
    assert 'rls_scored_total{result="done"} 1' in body
    assert 'rls_scoring_stage_seconds_count{stage="score"} 1' in body
    assert 'rls_scoring_stage_seconds_count{stage="leaderboard"} 1' in body
    assert 'rls_http_requests_total{method="POST",route="/v1/replays",status="202"} 1' in body


def test_dedupe_upload_counts_as_deduped(client, make_account):
    metrics.reset()
    make_account(credits_n=5)
    client.post("/v1/replays", files=_file(b"dupe"), headers=H)
    client.post("/v1/replays", files=_file(b"dupe"), headers=H)  # same bytes
    body = client.get("/metrics").text
    assert 'rls_uploads_total{outcome="accepted"} 1' in body
    assert 'rls_uploads_total{outcome="deduped"} 1' in body
