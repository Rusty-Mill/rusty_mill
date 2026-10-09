"""Replay-Scoring Service (M2).

The web/DB/monetization layer (spec §3–§9) over the Rust parse/feature/scoring
worker. The Rust binary stays the source of truth for *scoring*; this package
owns persistence, idempotency, the credit ledger, and the HTTP surface.
"""
