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


def build_pdf_naming_fonts_it_does_not_embed(
    fonts: dict[str, str] | None = None, width_ratio: float | None = None
) -> bytes:
    """A PDF that names fonts without carrying them.

    Plenty of real documents do this — a CV exported by a tool that assumes the
    reader has the font — and the viewer substitutes whatever it has. Such a
    file cannot be built with PyMuPDF's own writer, which always embeds, so it
    is assembled by hand.
    """
    fonts = fonts or {"F1": "DejaVuSerif-Bold", "F2": "DejaVuSerif", "F3": "Arial-BoldMT"}

    # With a widths table the file states how much room its text takes, which is
    # what a real document does and what the editor calibrates against. Setting
    # it to a known multiple of the stand-in's own widths makes the correction
    # the editor should arrive at known in advance.
    widths = b""
    if width_ratio is not None:
        reference = pymupdf.Font(fontfile=FONT_FILES["sans"])
        table = b" ".join(
            b"%d" % round(reference.text_length(chr(code), 1000) * width_ratio)
            for code in range(32, 127)
        )
        widths = b"/FirstChar 32/LastChar 126/Widths[" + table + b"]"
    content = b"".join(
        b"BT /%s 12 Tf 72 %d Td (Linea en %s) Tj ET\n"
        % (name.encode(), 760 - 30 * index, base.encode())
        for index, (name, base) in enumerate(fonts.items())
    )
    resources = b"".join(
        b"/%s %d 0 R" % (name.encode(), 5 + index) for index, name in enumerate(fonts)
    )
    objects = {
        1: b"<</Type/Catalog/Pages 2 0 R>>",
        2: b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        3: b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]"
           b"/Resources<</Font<<" + resources + b">>>>/Contents 4 0 R>>",
        4: b"<</Length %d>>\nstream\n" % len(content) + content + b"endstream",
    }
    for index, base in enumerate(fonts.values()):
        objects[5 + index] = (
            b"<</Type/Font/Subtype/TrueType/BaseFont/%s/Encoding/WinAnsiEncoding%s>>"
            % (base.encode(), widths)
        )

    out = bytearray(b"%PDF-1.4\n")
    offsets = {}
    for number in sorted(objects):
        offsets[number] = len(out)
        out += b"%d 0 obj " % number + objects[number] + b" endobj\n"
    start = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for number in sorted(objects):
        out += b"%010d 00000 n \n" % offsets[number]
    out += b"trailer <</Size %d/Root 1 0 R>>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objects) + 1, start,
    )
    return bytes(out)


def build_pdf_with_links() -> bytes:
    """A page whose text carries hyperlinks, as a CV or a report does."""
    doc = pymupdf.open()
    page = doc.new_page()
    for name, path in _available.items():
        page.insert_font(fontname=name, fontfile=path)
    # Two lines close enough, and even enough, to read as one paragraph, so
    # re-wrapping has something to work on.
    page.insert_text((72, 100), "Contacto: alguien@example.com y tambien su web",
                     fontname="serif", fontsize=12)
    page.insert_text((72, 115), "personal, con su propio enlace puesto justo aqui",
                     fontname="serif", fontsize=12)
    page.insert_text((72, 180), "Una linea suelta, lejos y sola", fontname="serif", fontsize=12)
    page.insert_link({"kind": pymupdf.LINK_URI, "from": pymupdf.Rect(120, 90, 220, 102),
                      "uri": "mailto:alguien@example.com"})
    page.insert_link({"kind": pymupdf.LINK_URI, "from": pymupdf.Rect(250, 90, 290, 102),
                      "uri": "https://example.com"})
    page.insert_link({"kind": pymupdf.LINK_URI, "from": pymupdf.Rect(200, 168, 280, 182),
                      "uri": "https://otra.example.com"})
    data = doc.tobytes(garbage=4, deflate=True)
    doc.close()
    return data


@pytest.fixture
def linked_doc():
    document = pymupdf.open(stream=build_pdf_with_links(), filetype="pdf")
    yield document
    document.close()


@pytest.fixture
def unembedded_doc():
    document = pymupdf.open(stream=build_pdf_naming_fonts_it_does_not_embed(), filetype="pdf")
    yield document
    document.close()


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
