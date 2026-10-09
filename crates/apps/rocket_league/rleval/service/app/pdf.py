"""Report PDF rendering (spec §4.10).

The Rust worker already emits a PDF-ready, self-contained report HTML; rendering
it to PDF is the only missing piece. Rendering is a port so the service is
testable without a PDF engine (and so the engine can be swapped). The production
adapter is weasyprint, imported lazily so the base install / the test suite don't
require its native libraries — install it with the ``pdf`` extra.
"""

from __future__ import annotations

from typing import Protocol


class PdfRenderError(RuntimeError):
    """The PDF engine is unavailable or failed to render."""


class PdfRenderer(Protocol):
    def render(self, html: str) -> bytes: ...


class WeasyPrintRenderer:
    """HTML → PDF via weasyprint (``pip install 'rls-service[pdf]'``)."""

    def render(self, html: str) -> bytes:
        try:
            from weasyprint import HTML  # lazy: optional native dependency
        except ImportError as e:  # pragma: no cover - exercised only without the dep
            raise PdfRenderError(
                "weasyprint is not installed; install the 'pdf' extra"
            ) from e
        try:
            return HTML(string=html).write_pdf()
        except Exception as e:  # noqa: BLE001 - surface any engine failure uniformly
            raise PdfRenderError(f"pdf render failed: {e}") from e
