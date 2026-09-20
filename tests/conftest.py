"""Fixtures: small PDFs built with real embedded fonts."""

from __future__ import annotations

import dataclasses
import os
import re
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


def build_rotated_paragraph(rotation: int, lines: int = 3) -> bytes:
    """A paragraph of running text set at a quarter turn."""
    steps = {0: (0, 14), 90: (14, 0), 180: (0, -14), 270: (-14, 0)}[rotation % 360]
    doc = pymupdf.open()
    page = doc.new_page()
    for name, path in _available.items():
        page.insert_font(fontname=name, fontfile=path)
    for index in range(lines):
        page.insert_text(
            pymupdf.Point(300 + steps[0] * index, 420 + steps[1] * index),
            " ".join(f"pal{index}{n}" for n in range(9)),
            fontname="serif",
            fontsize=11,
            rotate=rotation,
        )
    data = doc.tobytes(garbage=4, deflate=True)
    doc.close()
    return data


def build_pdf_with_section_below(gap: float = 20.0, top: float = 100.0) -> bytes:
    """A paragraph, a rule, and a section under it — a page with things in the way."""
    doc = pymupdf.open()
    page = doc.new_page()
    for name, path in _available.items():
        page.insert_font(fontname=name, fontfile=path)
    for index in range(3):
        page.insert_text(
            (72, top + 14 * index),
            " ".join(f"pal{index}{n}" for n in range(10)),
            fontname="serif",
            fontsize=11,
        )
    below = top + 28 + gap
    page.draw_line((72, below), (500, below), color=(0.2, 0.2, 0.2), width=0.8)
    page.insert_text((72, below + 20), "SECCION SIGUIENTE", fontname="serif-bold", fontsize=11)
    page.insert_text((72, below + 40), "contenido de la seccion", fontname="serif", fontsize=11)
    page.insert_link({
        "kind": pymupdf.LINK_URI,
        "from": pymupdf.Rect(72, below + 30, 200, below + 44),
        "uri": "https://ejemplo.com",
    })
    data = doc.tobytes(garbage=4, deflate=True)
    doc.close()
    return data


def build_pdf_with_figures(top: float = 100.0) -> bytes:
    """A paragraph with a curve, a quadrilateral and a picture under it.

    Everything below is inside the paragraph's own column, so all of it is in
    the way when the paragraph grows.
    """
    doc = pymupdf.open()
    page = doc.new_page()
    for name, path in _available.items():
        page.insert_font(fontname=name, fontfile=path)
    for index in range(3):
        page.insert_text(
            (72, top + 14 * index),
            " ".join(f"pal{index}{n}" for n in range(10)),
            fontname="serif",
            fontsize=11,
        )
    base = top + 70
    page.draw_bezier(
        (72, base), (120, base - 15), (180, base + 20), (240, base),
        color=(0.8, 0.1, 0.1), width=1.2,
    )
    page.draw_quad(
        pymupdf.Quad((80, base + 30), (200, base + 28), (82, base + 50), (202, base + 48)),
        color=(0, 0.4, 0), width=0.9,
    )
    picture = pymupdf.Pixmap(pymupdf.csRGB, pymupdf.IRect(0, 0, 60, 40))
    picture.set_rect(picture.irect, (30, 90, 160))
    page.insert_image(
        pymupdf.Rect(100, base + 60, 240, base + 120), stream=picture.tobytes("png")
    )
    page.insert_text((72, base + 160), "texto al final del bloque", fontname="serif", fontsize=11)
    data = doc.tobytes()
    doc.close()
    return data


def build_scanned_pdf(lines: list[str] | None = None) -> bytes:
    """A page that is a picture of text: what a scanner produces.

    Built by setting the text and then throwing the text away, keeping only the
    render — which is exactly what the editor cannot touch until it is read.
    """
    lines = lines or [
        "Informe de resultados trimestrales",
        "Este documento fue escaneado y no tiene capa de texto.",
        "El reconocimiento deberia recuperar estas lineas.",
    ]
    source = pymupdf.open()
    page = source.new_page(width=595, height=842)
    for name, path in _available.items():
        page.insert_font(fontname=name, fontfile=path)
    page.insert_text((72, 100), lines[0], fontname="serif", fontsize=18)
    for index, text in enumerate(lines[1:]):
        page.insert_text((72, 140 + index * 25), text, fontname="serif", fontsize=12)
    picture = page.get_pixmap(dpi=200)
    source.close()

    scan = pymupdf.open()
    sheet = scan.new_page(width=595, height=842)
    sheet.insert_image(sheet.rect, pixmap=picture)
    data = scan.tobytes()
    scan.close()
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


def build_pdf_with_a_form(pages: int = 2) -> bytes:
    """A PDF whose pages carry a filled-in text field, plus a highlight."""
    doc = pymupdf.open()
    for pno in range(pages):
        page = doc.new_page()
        for name, path in _available.items():
            page.insert_font(fontname=name, fontfile=path)
        page.insert_text((72, 100), f"Pagina {pno} del formulario",
                         fontname="helv", fontsize=12)
        widget = pymupdf.Widget()
        widget.field_name = f"campo{pno}"
        widget.field_type = pymupdf.PDF_WIDGET_TYPE_TEXT
        widget.rect = pymupdf.Rect(72, 300, 300, 320)
        widget.field_value = f"valor {pno}"
        page.add_widget(widget)
    doc[0].add_highlight_annot(pymupdf.Rect(70, 90, 260, 105))
    data = doc.tobytes()
    doc.close()
    return data


def form_fields(document: pymupdf.Document) -> list[tuple[str, str]]:
    """Every form field of a document, as (name, value)."""
    return [(w.field_name, w.field_value) for page in document for w in page.widgets()]


def build_pdf_with_marks() -> bytes:
    """A paragraph with a highlight over its first line, a note beside it, and
    a second paragraph further down that nothing marks."""
    doc = pymupdf.open()
    page = doc.new_page()
    for name, path in _available.items():
        page.insert_font(fontname=name, fontfile=path)
    page.insert_text((72, 100), "Primera linea del parrafo marcado", fontname="helv", fontsize=11)
    page.insert_text((72, 116), "Segunda linea del parrafo marcado", fontname="helv", fontsize=11)
    page.insert_text((72, 400), "Un parrafo sin marcar mas abajo", fontname="helv", fontsize=11)
    mark = page.add_highlight_annot(pymupdf.Rect(70, 90, 260, 105))
    mark.set_colors(stroke=(1.0, 0.9, 0.2))
    mark.set_info(content="sobre el parrafo")
    mark.update()
    rule = page.add_underline_annot(pymupdf.Rect(70, 106, 260, 120))
    rule.set_info(content="bajo el parrafo")
    rule.update()
    page.add_text_annot((300, 95), "al margen")
    other = page.add_highlight_annot(pymupdf.Rect(70, 390, 260, 405))
    other.set_info(content="lejos")
    other.update()
    data = doc.tobytes()
    doc.close()
    return data


def annots_of(document: pymupdf.Document, pno: int = 0) -> list[tuple[str, str, tuple]]:
    """Every annotation of a page as (type, note, rounded rect), in page order.

    The note is what tests address a mark by: rects move, and indices shift
    when a mark is taken off and put back.
    """
    return [
        (a.type[1], a.info.get("content", ""), tuple(round(v, 1) for v in a.rect))
        for a in document[pno].annots()
    ]


@dataclasses.dataclass
class AnnotSnapshot:
    """What an annotation looked like, read out while it was still valid.

    A live ``Annot`` belongs to the generator that yielded it; holding one past
    that and reading its rectangle segfaults, so tests get a copy instead.
    """

    kind: str
    note: str
    rect: tuple
    stroke: list | None
    opacity: float | None


def annot_named(document: pymupdf.Document, note: str, pno: int = 0) -> AnnotSnapshot | None:
    """The one annotation carrying ``note``, copied out of the page."""
    for annot in document[pno].annots():
        if annot.info.get("content") == note:
            return AnnotSnapshot(
                kind=annot.type[1],
                note=note,
                rect=tuple(round(v, 2) for v in annot.rect),
                stroke=annot.colors.get("stroke"),
                opacity=annot.opacity,
            )
    return None


def mark_quads(document: pymupdf.Document, pno: int = 0, index: int = 0) -> list:
    """The quadrilaterals of one text mark, read out where it is still valid."""
    for position, annot in enumerate(document[pno].annots()):
        if position == index:
            return list(annot.vertices or [])
    return []


def _append_to_array(doc, xref, key, refs):
    """Add object references to an array inside an object, by rewriting it.

    ``xref_set_key`` will not write an array into the catalogue — it stores a
    placeholder string instead — and the document's field list lives there,
    inline, with no xref of its own. So the whole object is read out and put
    back with the array extended.
    """
    raw = doc.xref_object(xref)
    added = " ".join(f"{ref} 0 R" for ref in refs)
    pattern = re.compile(rf"/{key}\s*\[")
    match = pattern.search(raw)
    assert match, f"no encontré /{key} en el objeto {xref}: {raw}"
    doc.update_object(xref, raw[: match.end()] + f" {added} " + raw[match.end():])


def build_pdf_with_every_kind_of_field() -> bytes:
    """A form carrying one of each field type this editor can meet.

    PyMuPDF will not build a radio group — its buttons hang off a parent field
    it has no interface for — so that part is written by hand, which is also
    the only way to get one with real appearance streams.
    """
    doc = pymupdf.open()
    page = doc.new_page()
    # A real form has labels beside its boxes, and the page has to carry text
    # for anything that edits text to have something to work on.
    for label, top in (("Nombre y apellidos", 96), ("Codigo corto", 126),
                       ("Referencia fija", 156), ("Acepto las condiciones", 196)):
        page.insert_text((320, top + 14), label, fontname="helv", fontsize=10)

    def widget(kind, name, rect, **rest):
        field = pymupdf.Widget()
        field.field_name = name
        field.field_type = kind
        field.rect = pymupdf.Rect(*rect)
        for key, value in rest.items():
            setattr(field, key, value)
        page.add_widget(field)

    widget(pymupdf.PDF_WIDGET_TYPE_TEXT, "nombre", (72, 100, 300, 120),
           field_value="Ana")
    widget(pymupdf.PDF_WIDGET_TYPE_TEXT, "corto", (72, 130, 300, 150),
           field_value="ab", text_maxlen=4)
    widget(pymupdf.PDF_WIDGET_TYPE_TEXT, "fijo", (72, 160, 300, 180),
           field_value="no tocar", field_flags=1)
    widget(pymupdf.PDF_WIDGET_TYPE_CHECKBOX, "acepto", (72, 200, 90, 218),
           field_value=False)
    widget(pymupdf.PDF_WIDGET_TYPE_COMBOBOX, "pais", (72, 240, 250, 260),
           choice_values=["España", "Francia", "Portugal"], field_value="España")
    widget(pymupdf.PDF_WIDGET_TYPE_LISTBOX, "plan", (72, 280, 250, 320),
           choice_values=["Basico", "Pro"], field_value="Basico")
    widget(pymupdf.PDF_WIDGET_TYPE_SIGNATURE, "firma", (72, 340, 250, 380))

    doc = pymupdf.open(stream=doc.tobytes(), filetype="pdf")
    page = doc[0]

    def appearance(marked):
        xref = doc.get_new_xref()
        doc.update_object(xref, "<< /Type /XObject /Subtype /Form /BBox [0 0 18 18] >>")
        doc.update_stream(xref, b"q 0 0 1 rg 4 4 10 10 re f Q" if marked else b" ", new=True)
        return xref

    parent = doc.get_new_xref()
    kids = []
    for state, top in (("Si", 420), ("No", 450), ("NS", 480)):
        kid = doc.get_new_xref()
        doc.update_object(kid, f"""<< /Type /Annot /Subtype /Widget
            /Rect [72 {top} 90 {top + 18}] /FT /Btn /Ff 32768 /Parent {parent} 0 R
            /AS /Off /AP << /N << /{state} {appearance(True)} 0 R
            /Off {appearance(False)} 0 R >> >> >>""")
        kids.append(kid)
    doc.update_object(parent, f"""<< /FT /Btn /Ff 32768 /T (respuesta) /V /Off
        /Kids [{" ".join(f"{kid} 0 R" for kid in kids)}] >>""")

    _append_to_array(doc, page.xref, "Annots", kids)
    _append_to_array(doc, doc.pdf_catalog(), "Fields", [parent])

    data = doc.tobytes()
    doc.close()
    return data


def field_named(document: pymupdf.Document, name: str, pno: int = 0, which: int = 0) -> dict:
    """One field of a page, by name; `which` picks among a radio group."""
    from app.forms import page_fields

    matches = [f for f in page_fields(document[pno]) if f["name"] == name]
    return matches[which] if len(matches) > which else None


def sign_field(doc: pymupdf.Document, xref: int, name: str | None = "Cesar Ruiz") -> None:
    """Make a signature field read as signed, without a real certificate.

    ``is_signed`` reads whether the field's ``/V`` looks like a completed
    signature dictionary — filter, byte range, contents — not whether it
    cryptographically checks out, so a plausible-looking one is enough to
    exercise the warning this editor gives. ``name=None`` signs it without
    recording who by, the way a signature that carries no ``/Name`` would.
    """
    who = f" /Name ({name})" if name else ""
    doc.xref_set_key(
        xref, "V",
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached "
        "/ByteRange [0 10 20 30] /Contents <" + "00" * 32 + f">{who} >>",
    )
