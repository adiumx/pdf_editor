"""Reading the text off a scanned page, and editing it afterwards."""

from __future__ import annotations

import pymupdf
import pytest

from app.editor import apply_operations
from app.extract import extract_page, page_summaries
from app.fonts import FontResolver, is_placeholder
from app.ocr import needs_ocr, recognise, support, was_recognised
from tests.conftest import build_pdf, build_scanned_pdf, requires_fonts

requires_ocr = pytest.mark.skipif(
    not support().available,
    reason=f"el reconocimiento no está disponible: {support().detail}",
)


def ink(page: pymupdf.Page, rect) -> int:
    """How many dark pixels a region has — what a reader would see there."""
    pixmap = page.get_pixmap(clip=pymupdf.Rect(rect), dpi=150)
    return sum(1 for i in range(0, len(pixmap.samples), pixmap.n) if pixmap.samples[i] < 200)


@pytest.fixture
def scanned():
    document = pymupdf.open(stream=build_scanned_pdf(), filetype="pdf")
    yield document
    document.close()


class TestSupport:
    def test_it_says_plainly_whether_it_can_work(self):
        capability = support()
        assert isinstance(capability.available, bool)
        if not capability.available:
            assert capability.detail, "debería explicar qué falta"
        else:
            assert capability.languages
            assert capability.default_language in capability.languages


@requires_fonts
class TestDetection:
    def test_a_scan_is_recognised_as_one(self, scanned):
        assert needs_ocr(scanned[0]) is True
        assert scanned[0].get_text().strip() == ""

    def test_an_ordinary_document_is_not(self):
        document = pymupdf.open(stream=build_pdf(), filetype="pdf")
        assert needs_ocr(document[0]) is False
        document.close()

    def test_pages_report_their_state(self, scanned):
        summary = page_summaries(scanned)[0]
        assert summary["needs_ocr"] is True
        assert summary["recognised"] is False


@requires_fonts
@requires_ocr
class TestRecognising:
    def test_the_text_comes_back(self, scanned):
        assert recognise(scanned) == [0]
        text = scanned[0].get_text()
        assert "Informe" in text and "escaneado" in text

    def test_the_page_keeps_its_size(self, scanned):
        before = pymupdf.Rect(scanned[0].rect)
        recognise(scanned)
        assert list(scanned[0].rect) == pytest.approx(list(before), abs=1.0)

    def test_the_page_is_marked_so_editing_knows(self, scanned):
        """The mark lives in the page itself, so reordering or saving keeps it."""
        recognise(scanned)
        assert was_recognised(scanned[0]) is True
        saved = pymupdf.open(stream=scanned.tobytes(), filetype="pdf")
        assert was_recognised(saved[0]) is True
        saved.close()

    def test_it_does_not_run_twice(self, scanned):
        recognise(scanned)
        assert needs_ocr(scanned[0]) is False
        assert page_summaries(scanned)[0]["needs_ocr"] is False

    def test_words_become_lines_you_can_edit(self, scanned):
        """Recognition emits a run per word; editing one word at a time is not
        editing a document."""
        recognise(scanned)
        page = extract_page(scanned, 0, FontResolver(scanned))
        assert page["lines"]
        for line in page["lines"]:
            assert len(line["spans"]) == 1, f"{line['text']!r} quedó en trozos"


@requires_fonts
@requires_ocr
class TestEditingAScan:
    def _line(self, document, needle="Informe"):
        page = extract_page(document, 0, FontResolver(document))
        return next(line for line in page["lines"] if needle in line["text"])

    def test_the_scanned_pixels_are_blanked(self, scanned):
        """The old word is a picture. Erasing only the text layer would leave it
        showing through whatever replaces it."""
        recognise(scanned)
        line = self._line(scanned)
        before = ink(scanned[0], line["bbox"])
        assert before > 500, "el escaneo debería tener tinta ahí"

        spans = [dict(span) for span in line["spans"]]
        spans[0]["text"] = "X"
        apply_operations(scanned, FontResolver(scanned), [{
            "op": "replace_line", "page": 0, "bbox": line["bbox"],
            "origin": line["origin"], "rotation": line["rotation"],
            "align": line["align"], "spans": spans,
        }])
        assert "trimestrales" not in scanned[0].get_text()

    def test_the_replacement_is_visible(self, scanned):
        """The recognised layer is invisible on purpose; inheriting that would
        erase the line and put nothing in its place."""
        recognise(scanned)
        line = self._line(scanned)
        spans = [dict(span) for span in line["spans"]]
        spans[0]["text"] = "Informe ANUAL de resultados"
        apply_operations(scanned, FontResolver(scanned), [{
            "op": "replace_line", "page": 0, "bbox": line["bbox"],
            "origin": line["origin"], "rotation": line["rotation"],
            "align": line["align"], "spans": spans,
        }])
        assert ink(scanned[0], line["bbox"]) > 500, "el texto nuevo no se ve"
        assert "Informe ANUAL" in scanned[0].get_text()

    def test_the_placeholder_font_is_never_reused(self, scanned):
        """It covers every character and draws none of them: picked as a
        perfect match, it would replace the text with blanks."""
        recognise(scanned)
        resolver = FontResolver(scanned)
        resolved = resolver.resolve(0, "GlyphLessFont", 12, "Informe")
        assert resolved.source != "embedded"
        assert not is_placeholder(resolved.font.name)

    def test_the_placeholder_is_not_offered_as_a_choice(self, scanned):
        recognise(scanned)
        families = FontResolver(scanned).available_families()
        assert not any(is_placeholder(family["name"]) for family in families)

    def test_its_flags_are_not_believed(self, scanned):
        """Recognition marks its placeholder monospaced and serifed. Believing
        that sets the replacement in a typewriter face."""
        recognise(scanned)
        resolved = FontResolver(scanned).resolve(0, "GlyphLessFont", 12, "Informe")
        assert "mono" not in resolved.font.name.lower()

    def test_nothing_is_calibrated_against_it(self, scanned):
        """Its recorded widths say where words sit in a picture, not what a
        font measures; correcting to them stretches the replacement."""
        recognise(scanned)
        resolver = FontResolver(scanned)
        resolver.scan_page(0)
        assert resolver.resolve(0, "GlyphLessFont", 12, "Informe").hscale == 1.0


@requires_fonts
class TestOcrApi:
    @pytest.fixture
    def opened(self):
        from fastapi.testclient import TestClient

        from app.main import app, store

        with TestClient(app) as client:
            response = client.post(
                "/api/documents",
                files={"file": ("escaneado.pdf", build_scanned_pdf(), "application/pdf")},
            )
            yield client, response.json()
        store.close_all()

    def test_the_document_says_whether_recognition_is_possible(self, opened):
        _client, state = opened
        assert "ocr" in state
        assert isinstance(state["ocr"]["available"], bool)
        assert state["pages"][0]["needs_ocr"] is True

    @requires_ocr
    def test_recognising_makes_the_page_editable(self, opened):
        client, state = opened
        body = client.post(f"/api/documents/{state['id']}/ocr", json={}).json()
        assert body["recognised"] == [0]
        assert body["pages"][0]["recognised"] is True

        page = client.get(f"/api/documents/{state['id']}/pages/0/text").json()
        assert any("Informe" in line["text"] for line in page["lines"])

    @requires_ocr
    def test_it_is_one_undoable_step(self, opened):
        client, state = opened
        client.post(f"/api/documents/{state['id']}/ocr", json={})
        client.post(f"/api/documents/{state['id']}/undo")
        page = client.get(f"/api/documents/{state['id']}/pages/0/text").json()
        assert page["lines"] == []
