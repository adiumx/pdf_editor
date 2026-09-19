"""Fixtures: small PDFs built with real embedded fonts."""

from __future__ import annotations

import os
import sys

import pymupdf
import pytest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from app.fonts import FontResolver  # noqa: E402

FONT_FILES = {
    "serif": "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
    "serif-bold": "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
    "sans": "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
}

_available = {name: path for name, path in FONT_FILES.items() if os.path.isfile(path)}

requires_fonts = pytest.mark.skipif(
    len(_available) < len(FONT_FILES),
    reason="las fuentes Liberation no están instaladas en este equipo",
)


def build_pdf(
    lines: list[tuple[tuple[float, float], str, str, float]] | None = None,
    *,
    pages: int = 1,
    rotation: int = 0,
) -> bytes:
    """A PDF with `lines` of (origin, text, font key, size) on its first page."""
    lines = lines or [
        ((72, 100), "Primera linea del documento", "serif", 12),
        ((72, 118), "Segunda linea justo debajo", "serif", 12),
        ((72, 140), "Titulo en negrita", "serif-bold", 16),
        ((72, 170), "Una linea en sans serif", "sans", 12),
    ]
    doc = pymupdf.open()
    page = doc.new_page()
    for name, path in _available.items():
        page.insert_font(fontname=name, fontfile=path)
    for origin, text, font, size in lines:
        page.insert_text(origin, text, fontname=font, fontsize=size)
    if rotation:
        page.set_rotation(rotation)
    for _ in range(pages - 1):
        extra = doc.new_page()
        extra.insert_text((72, 100), "Pagina adicional", fontsize=12)
    data = doc.tobytes(garbage=4, deflate=True)
    doc.close()
    return data


@pytest.fixture
def pdf_bytes() -> bytes:
    return build_pdf()


@pytest.fixture
def doc(pdf_bytes: bytes):
    document = pymupdf.open(stream=pdf_bytes, filetype="pdf")
    yield document
    document.close()


@pytest.fixture
def resolver(doc):
    return FontResolver(doc)


def spans_of(document: pymupdf.Document, pno: int = 0) -> list[dict]:
    """Every span of a page, flattened, straight from PyMuPDF."""
    return [
        span
        for block in document[pno].get_text("dict")["blocks"]
        if block.get("type") == 0
        for line in block["lines"]
        for span in line["spans"]
    ]


def text_of(document: pymupdf.Document, pno: int = 0) -> str:
    return document[pno].get_text().strip()
