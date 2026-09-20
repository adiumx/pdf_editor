"""Reading a page back out: geometry, font data, alignment, rotation."""

from __future__ import annotations

import pymupdf
import pytest

from app.extract import (
    area_contents,
    color_to_hex,
    extract_page,
    hex_to_pdf,
    page_summaries,
    rotation_from_dir,
)
from app.fonts import FontResolver
from tests.conftest import (
    build_pdf,
    build_pdf_with_every_kind_of_field,
    build_pdf_with_figures,
    build_pdf_with_marks,
    requires_fonts,
)


class TestConversions:
    def test_colour_round_trips_through_hex(self):
        assert color_to_hex(0x336699) == "#336699"
        assert hex_to_pdf("#ffffff") == (1.0, 1.0, 1.0)
        assert hex_to_pdf("#000") == (0.0, 0.0, 0.0)

    def test_a_malformed_colour_is_read_as_black(self):
        assert hex_to_pdf("no-es-un-color") == (0.0, 0.0, 0.0)

    @pytest.mark.parametrize(
        ("direction", "degrees"),
        [((1, 0), 0), ((0, -1), 90), ((-1, 0), 180), ((0, 1), 270)],
    )
    def test_direction_cosines_become_degrees(self, direction, degrees):
        assert rotation_from_dir(direction) == degrees


@requires_fonts
class TestExtraction:
    def test_every_span_carries_what_it_takes_to_redraw_it(self, doc, resolver):
        page = extract_page(doc, 0, resolver)
        assert page["lines"]
        for line in page["lines"]:
            assert line["bbox"] and line["origin"]
            for span in line["spans"]:
                assert span["text"]
                assert span["font"]
                assert span["size"] > 0
                assert span["color"].startswith("#")
                assert span["embedded"] is True

    def test_geometry_matches_the_page(self, doc, resolver):
        page = extract_page(doc, 0, resolver)
        assert page["width"] == pytest.approx(doc[0].rect.width, abs=0.01)
        assert page["height"] == pytest.approx(doc[0].rect.height, abs=0.01)

    def test_bold_and_size_survive_extraction(self, doc, resolver):
        page = extract_page(doc, 0, resolver)
        spans = [span for line in page["lines"] for span in line["spans"]]
        heading = next(span for span in spans if "negrita" in span["text"])
        assert heading["bold"] is True
        assert heading["size"] == pytest.approx(16, abs=0.1)

    def test_right_aligned_block_is_detected_as_right_aligned(self):
        """Three lines sharing a right edge are right-aligned, not left."""
        font = pymupdf.Font(fontfile="/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf")
        lines = []
        for index, text in enumerate(["Total: 1.240,00", "IVA: 260,40", "A pagar: 1.500,40"]):
            width = font.text_length(text, 12)
            lines.append(((500 - width, 300 + index * 15), text, "sans", 12))
        document = pymupdf.open(stream=build_pdf(lines), filetype="pdf")
        page = extract_page(document, 0, FontResolver(document))
        aligns = {line["align"] for line in page["lines"] if "Total" in line["text"] or "IVA" in line["text"]}
        assert aligns == {"right"}
        document.close()

    @pytest.mark.parametrize("rotation", [0, 90, 180, 270])
    def test_coordinates_stay_inside_the_rendered_page(self, rotation):
        """The client positions overlays on the rendered image, so extracted
        boxes have to live in the rendered page's coordinate space."""
        document = pymupdf.open(stream=build_pdf(rotation=rotation), filetype="pdf")
        page = extract_page(document, 0, FontResolver(document))
        rect = document[0].rect
        for line in page["lines"]:
            x0, y0, x1, y1 = line["bbox"]
            assert -1 <= x0 <= x1 <= rect.width + 1
            assert -1 <= y0 <= y1 <= rect.height + 1
        document.close()

    def test_page_summaries_cover_every_page(self):
        document = pymupdf.open(stream=build_pdf(pages=3), filetype="pdf")
        summaries = page_summaries(document)
        assert [item["page"] for item in summaries] == [0, 1, 2]
        document.close()


@requires_fonts
class TestColumnMeasure:
    """A PDF records no margins, so how far a paragraph's text may run has to be
    inferred. Its own widest line is a poor guess: a short paragraph would be
    re-wrapped into a column narrower than the one it sits in."""

    def _page(self):
        """A wide paragraph and, at the same margin, a much shorter one."""
        long_line = " ".join(f"palabra{n:02d}" for n in range(12))
        rows = [
            ((72, 100), long_line, "serif", 11),
            ((72, 114), long_line, "serif", 11),
            ((72, 160), "corto uno", "serif", 11),
            ((72, 174), "corto dos", "serif", 11),
        ]
        document = pymupdf.open(stream=build_pdf(rows), filetype="pdf")
        return document, extract_page(document, 0, FontResolver(document))

    def test_a_short_paragraph_takes_the_measure_of_its_column(self):
        document, page = self._page()
        short = next(line for line in page["lines"] if "corto" in line["text"])
        assert short["measure"] > short["block_bbox"][2] + 10
        document.close()

    def test_the_measure_never_falls_below_the_paragraphs_own_width(self):
        document, page = self._page()
        for line in page["lines"]:
            assert line["measure"] >= line["block_bbox"][2] - 0.01
        document.close()

    def test_a_different_margin_gets_its_own_measure(self):
        """An indented block is its own column, not the body's."""
        rows = [
            ((72, 100), " ".join(f"ancho{n:02d}" for n in range(12)), "serif", 11),
            ((300, 140), "bloque aparte", "serif", 11),
        ]
        document = pymupdf.open(stream=build_pdf(rows), filetype="pdf")
        page = extract_page(document, 0, FontResolver(document))
        apart = next(line for line in page["lines"] if "aparte" in line["text"])
        wide = next(line for line in page["lines"] if "ancho" in line["text"])
        assert apart["measure"] < wide["measure"]
        document.close()


class TestSayingWhatAnEraseTakes:
    """Erasing is not a covering-up, so what it takes is read back first."""

    def _page(self):
        doc = pymupdf.open(stream=build_pdf_with_marks(), filetype="pdf")
        return doc, doc[0]

    def test_the_text_inside_comes_back(self):
        doc, page = self._page()
        inside = area_contents(page, pymupdf.Rect(70, 88, 400, 125))
        assert "Primera linea del parrafo marcado" in inside["text"]
        assert inside["words"] == 10
        doc.close()

    def test_text_outside_is_not_counted(self):
        doc, page = self._page()
        inside = area_contents(page, pymupdf.Rect(70, 88, 400, 125))
        assert "sin marcar" not in inside["text"]
        doc.close()

    def test_an_empty_area_says_so(self):
        doc, page = self._page()
        assert area_contents(page, pymupdf.Rect(400, 600, 500, 650))["empty"] is True
        doc.close()

    def test_an_area_with_text_is_not_empty(self):
        doc, page = self._page()
        assert area_contents(page, pymupdf.Rect(70, 88, 400, 125))["empty"] is False
        doc.close()

    def test_the_marks_that_will_stay_are_counted(self):
        """A redaction takes page content; a highlight over it is not page
        content and survives, left pointing at the gap."""
        doc, page = self._page()
        assert area_contents(page, pymupdf.Rect(70, 88, 400, 125))["marks"] == 3
        doc.close()

    def test_an_annotations_own_paint_is_not_counted_as_line_art(self):
        """It is reported among the page's drawings with nothing to tell the
        two apart, and a redaction does not touch it anyway."""
        doc, page = self._page()
        assert area_contents(page, pymupdf.Rect(70, 88, 400, 125))["drawings"] == 0
        doc.close()

    def test_real_line_art_is_counted(self):
        doc = pymupdf.open()
        page = doc.new_page()
        page.insert_text((72, 100), "Una linea", fontname="helv", fontsize=11)
        page.draw_rect(pymupdf.Rect(80, 150, 200, 190))
        page.add_highlight_annot(pymupdf.Rect(70, 90, 200, 105))
        inside = area_contents(page, pymupdf.Rect(60, 80, 300, 250))
        assert inside["drawings"] == 1, "no distinguió el recuadro del resaltado"
        assert inside["marks"] == 1
        doc.close()

    def test_a_form_field_over_the_area_is_counted_as_staying(self):
        doc = pymupdf.open(stream=build_pdf_with_every_kind_of_field(), filetype="pdf")
        inside = area_contents(doc[0], pymupdf.Rect(60, 90, 320, 200))
        assert inside["fields"] >= 3
        doc.close()

    def test_an_image_inside_is_counted(self):
        doc = pymupdf.open(stream=build_pdf_with_figures(), filetype="pdf")
        whole = area_contents(doc[0], doc[0].rect)
        assert whole["images"] >= 1
        doc.close()
