"""Applying edits: does the text change, and does nothing else?"""

from __future__ import annotations

import pymupdf
import pytest

from app.editor import _REDACT_PAD, EditError, EditSession, apply_operations
from app.extract import extract_page
from app.fonts import FontResolver
from tests.conftest import (
    FONT_FILES,
    build_pdf,
    build_pdf_with_links,
    requires_fonts,
    spans_of,
    text_of,
)


def edit_line(doc, line, text, **extra):
    """Replace a whole line's first span with `text`, as the client would."""
    spans = [dict(span) for span in line["spans"]]
    spans[0]["text"] = text
    operation = {
        "op": "replace_line",
        "page": line["page"],
        "bbox": line["bbox"],
        "origin": line["origin"],
        "rotation": line["rotation"],
        "align": line.get("align", "left"),
        "spans": spans,
        **extra,
    }
    return apply_operations(doc, FontResolver(doc), [operation])


def line_named(doc, needle, resolver=None):
    page = extract_page(doc, 0, resolver or FontResolver(doc))
    return next(line for line in page["lines"] if needle in line["text"])


@requires_fonts
class TestReplacingText:
    def test_the_replacement_keeps_font_size_and_colour(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        before = line_named(doc, "Primera linea")["spans"][0]
        edit_line(doc, line_named(doc, "Primera linea"), "Texto completamente nuevo")
        after = line_named(doc, "Texto completamente")["spans"][0]

        assert after["font"] == before["font"]
        assert after["size"] == pytest.approx(before["size"], abs=0.01)
        assert after["color"] == before["color"]
        assert after["embedded"] is True
        doc.close()

    def test_the_replacement_sits_on_the_original_baseline(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        before = line_named(doc, "Primera linea")
        edit_line(doc, before, "Otro texto")
        after = line_named(doc, "Otro texto")
        assert after["origin"][1] == pytest.approx(before["origin"][1], abs=0.5)
        assert after["bbox"][0] == pytest.approx(before["bbox"][0], abs=0.5)
        doc.close()

    def test_editing_a_line_leaves_the_line_below_it_untouched(self):
        """Redaction rectangles must not reach into neighbouring lines."""
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        neighbours = [
            line["text"] for line in extract_page(doc, 0, FontResolver(doc))["lines"]
            if "Primera linea" not in line["text"]
        ]
        edit_line(doc, line_named(doc, "Primera linea"), "Cambiada")
        after = [line["text"] for line in extract_page(doc, 0, FontResolver(doc))["lines"]]
        for neighbour in neighbours:
            assert neighbour in after, f"la edición se comió «{neighbour}»"
        doc.close()

    def test_accented_and_symbol_characters_survive(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        edit_line(doc, line_named(doc, "Primera linea"), "Año: 1.500,40 € — ñáéíóú ¿qué?")
        assert "Año" in text_of(doc)
        assert "€" in text_of(doc)
        assert "ñáéíóú" in text_of(doc)
        doc.close()

    def test_unrepresentable_characters_are_reported_not_silently_dropped(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        warnings = edit_line(doc, line_named(doc, "Primera linea"), "日本語のテキスト")
        assert any(w.kind in {"font-substituted", "missing-glyphs"} for w in warnings)
        doc.close()

    def test_shrink_keeps_longer_text_inside_the_original_width(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        original = line_named(doc, "Primera linea")
        edit_line(doc, original, "Primera linea del documento algo mas larga", fit="shrink")
        after = line_named(doc, "Primera linea del documento algo")
        original_width = original["bbox"][2] - original["bbox"][0]
        assert after["bbox"][2] - after["bbox"][0] <= original_width + 1
        assert after["spans"][0]["size"] < original["spans"][0]["size"]
        doc.close()

    def test_shrink_stops_before_the_text_becomes_illegible(self):
        """Past a point, overflowing is better than microscopic text — and the
        user is told rather than left to discover it."""
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        original = line_named(doc, "Primera linea")
        warnings = edit_line(
            doc, original,
            "Un texto muchisimo mas largo que el que habia originalmente aqui puesto",
            fit="shrink",
        )
        after = line_named(doc, "Un texto muchisimo")
        assert after["spans"][0]["size"] >= original["spans"][0]["size"] * 0.5 - 0.01
        assert any(w.kind == "overflow" for w in warnings)
        doc.close()

    def test_overflow_is_allowed_but_warned_about(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        warnings = edit_line(
            doc, line_named(doc, "Primera linea"),
            "Un texto mucho mas largo que el que habia originalmente aqui",
        )
        assert any(w.kind == "overflow" for w in warnings)
        doc.close()

    def test_right_aligned_text_stays_anchored_to_its_right_edge(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        original = line_named(doc, "Primera linea")
        edit_line(doc, original, "Corto", align="right")
        after = line_named(doc, "Corto")
        assert after["bbox"][2] == pytest.approx(original["bbox"][2], abs=1.0)
        doc.close()

    @pytest.mark.parametrize("rotation", [0, 90, 180, 270])
    def test_edits_land_correctly_on_a_rotated_page(self, rotation):
        doc = pymupdf.open(stream=build_pdf(rotation=rotation), filetype="pdf")
        original = line_named(doc, "Primera linea")
        edit_line(doc, original, "Rotado y cambiado")
        after = line_named(doc, "Rotado y cambiado")
        assert after["rotation"] == original["rotation"]
        rect = doc[0].rect
        assert 0 <= after["bbox"][0] <= rect.width
        assert 0 <= after["bbox"][1] <= rect.height
        doc.close()


@requires_fonts
class TestOtherOperations:
    def test_deleting_a_line_removes_only_that_line(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        line = line_named(doc, "Primera linea")
        apply_operations(doc, FontResolver(doc), [{
            "op": "delete_line", "page": 0, "bbox": line["bbox"],
            "origin": line["origin"], "rotation": line["rotation"],
        }])
        remaining = text_of(doc)
        assert "Primera linea" not in remaining
        assert "Segunda linea" in remaining
        doc.close()

    def test_added_text_appears_with_the_requested_size(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        apply_operations(doc, FontResolver(doc), [{
            "op": "add_text", "page": 0, "rect": [72, 400, 400, 440],
            "text": "Texto nuevo insertado", "size": 14, "color": "#cc0000", "family": "sans",
        }])
        span = next(s for s in spans_of(doc) if "Texto nuevo" in s["text"])
        assert span["size"] == pytest.approx(14, abs=0.1)
        assert span["color"] == 0xCC0000
        doc.close()

    def test_added_multiline_text_becomes_several_lines(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        apply_operations(doc, FontResolver(doc), [{
            "op": "add_text", "page": 0, "rect": [72, 400, 400, 460],
            "text": "Primera\nSegunda\nTercera", "size": 12,
        }])
        text = text_of(doc)
        assert "Primera" in text and "Segunda" in text and "Tercera" in text
        doc.close()

    def test_erasing_an_area_clears_the_text_inside_it(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        apply_operations(doc, FontResolver(doc), [
            {"op": "erase_area", "page": 0, "rect": [60, 85, 400, 125]},
        ])
        text = text_of(doc)
        assert "Primera linea" not in text
        assert "Titulo en negrita" in text
        doc.close()

    def test_a_whole_batch_is_applied_at_once(self):
        """Two edits on one page must not erase each other's new text."""
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        page = extract_page(doc, 0, FontResolver(doc))
        operations = []
        for line, replacement in zip(page["lines"][:2], ["Alfa cambiada", "Beta cambiada"]):
            spans = [dict(span) for span in line["spans"]]
            spans[0]["text"] = replacement
            operations.append({
                "op": "replace_line", "page": 0, "bbox": line["bbox"],
                "origin": line["origin"], "rotation": line["rotation"],
                "align": line["align"], "spans": spans,
            })
        apply_operations(doc, FontResolver(doc), operations)
        text = text_of(doc)
        assert "Alfa cambiada" in text
        assert "Beta cambiada" in text
        doc.close()

    def test_an_image_can_be_placed_on_the_page(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        pixmap = pymupdf.Pixmap(pymupdf.csRGB, pymupdf.IRect(0, 0, 40, 40))
        pixmap.set_rect(pixmap.irect, (200, 30, 30))
        before = len(doc[0].get_images())
        apply_operations(doc, FontResolver(doc), [
            {"op": "insert_image", "page": 0, "rect": [300, 400, 400, 500], "asset": "a1"},
        ], {"a1": pixmap.tobytes("png")})
        assert len(doc[0].get_images()) == before + 1
        doc.close()

    def test_a_missing_image_asset_is_an_error_not_a_crash(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        with pytest.raises(EditError):
            apply_operations(doc, FontResolver(doc), [
                {"op": "insert_image", "page": 0, "rect": [0, 0, 10, 10], "asset": "desaparecida"},
            ], {})
        doc.close()

    def test_an_unknown_operation_is_rejected(self, doc, resolver):
        with pytest.raises(EditError):
            apply_operations(doc, resolver, [{"op": "formatear_el_disco"}])

    def test_an_edit_to_a_page_that_does_not_exist_is_rejected(self, doc, resolver):
        with pytest.raises(EditError):
            apply_operations(doc, resolver, [
                {"op": "replace_line", "page": 99, "bbox": [0, 0, 10, 10], "spans": []},
            ])


@requires_fonts
class TestPageOperations:
    def test_pages_can_be_reordered(self):
        doc = pymupdf.open(stream=build_pdf(pages=3), filetype="pdf")
        first = doc[0].get_text()
        apply_operations(doc, FontResolver(doc), [{"op": "move_page", "page": 0, "to": 3}])
        assert doc.page_count == 3
        assert doc[2].get_text() == first
        doc.close()

    def test_a_page_can_be_deleted(self):
        doc = pymupdf.open(stream=build_pdf(pages=3), filetype="pdf")
        apply_operations(doc, FontResolver(doc), [{"op": "delete_page", "page": 1}])
        assert doc.page_count == 2
        doc.close()

    def test_the_last_page_cannot_be_deleted(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        with pytest.raises(EditError):
            apply_operations(doc, FontResolver(doc), [{"op": "delete_page", "page": 0}])
        doc.close()

    def test_a_page_can_be_rotated(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        apply_operations(doc, FontResolver(doc), [
            {"op": "rotate_page", "page": 0, "rotation": 90},
        ])
        assert doc[0].rotation == 90
        doc.close()

    def test_a_blank_page_can_be_added(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        apply_operations(doc, FontResolver(doc), [{"op": "insert_page", "at": 1}])
        assert doc.page_count == 2
        assert doc[1].get_text().strip() == ""
        doc.close()


@requires_fonts
class TestLinksSurviveEditing:
    """Redaction takes a page's links with it. A CV edited here used to come
    out with its email and repository links silently gone."""

    def _edit(self, doc, needle, replacement):
        line = line_named(doc, needle)
        return edit_line(doc, line, replacement), line

    def test_a_links_on_an_edited_line_are_kept(self, linked_doc):
        before = {link["uri"] for link in linked_doc[0].get_links()}
        self._edit(linked_doc, "Contacto", "Contacto: otra direccion distinta")
        after = {link["uri"] for link in linked_doc[0].get_links()}
        assert before == after, f"desaparecieron {before - after}"

    def test_links_on_other_lines_are_untouched(self, linked_doc):
        original = next(
            link for link in linked_doc[0].get_links() if "otra" in link["uri"]
        )
        self._edit(linked_doc, "Contacto", "Texto nuevo")
        survivor = next(
            link for link in linked_doc[0].get_links() if "otra" in link["uri"]
        )
        assert list(survivor["from"]) == pytest.approx(list(original["from"]), abs=0.1)

    def test_a_link_follows_the_text_that_replaced_it(self, linked_doc):
        """A line that grows or shrinks carries its links along with it."""
        before = next(
            link for link in linked_doc[0].get_links() if link["uri"].startswith("mailto")
        )
        self._edit(linked_doc, "Contacto", "Contacto breve")
        after = next(
            link for link in linked_doc[0].get_links() if link["uri"].startswith("mailto")
        )
        assert list(after["from"]) != pytest.approx(list(before["from"]), abs=0.1)
        line = line_named(linked_doc, "Contacto breve")
        assert after["from"][0] >= line["bbox"][0] - 1
        assert after["from"][2] <= line["bbox"][2] + 1

    def test_deleting_a_line_drops_its_link(self, linked_doc):
        """Nothing is left to click, so putting the link back would be wrong."""
        line = line_named(linked_doc, "Contacto")
        apply_operations(linked_doc, FontResolver(linked_doc), [{
            "op": "delete_line", "page": 0, "bbox": line["bbox"],
            "origin": line["origin"], "rotation": line["rotation"],
        }])
        remaining = {link["uri"] for link in linked_doc[0].get_links()}
        assert not any(uri.startswith("mailto") for uri in remaining)
        assert any("otra" in uri for uri in remaining)


@requires_fonts
class TestUntouchedSpansAreLeftAlone:
    """Rewriting a whole line re-renders spans nobody edited. On a document
    whose fonts cannot be reused that means untouched words change typeface, so
    the part before the edit is left on the page as it was.

    What that comes down to is where the erasure starts, so that is what these
    check: the text assertions alone cannot tell the two paths apart, because a
    left-aligned line redrawn in full puts its first span back where it was.
    """

    def _mixed_line(self):
        """A bold label followed by ordinary text, as one line.

        The second run has to start exactly where the first ends, or extraction
        reads them as two separate lines and there is no mixed line to test.
        """
        label = "Etiqueta:"
        width = pymupdf.Font(fontfile=FONT_FILES["serif-bold"]).text_length(label, 12)
        doc = pymupdf.open(stream=build_pdf([
            ((72, 100), label, "serif-bold", 12),
            ((72 + width, 100), "valor que si se edita", "serif", 12),
        ]), filetype="pdf")
        page = extract_page(doc, 0, FontResolver(doc))
        line = max(page["lines"], key=lambda item: len(item["spans"]))
        assert len(line["spans"]) >= 2, "el PDF de prueba no produjo una línea mixta"
        return doc, line

    def _redaction_for(self, doc, line, align, from_span):
        """The rectangle an edit would erase, without applying it."""
        session = EditSession(doc, FontResolver(doc))
        spans = [dict(span) for span in line["spans"]]
        spans[1]["text"] = "otro valor"
        session.replace_line({
            "op": "replace_line", "page": 0, "bbox": line["bbox"], "origin": line["origin"],
            "rotation": line["rotation"], "align": align, "from_span": from_span,
            "spans": spans,
        })
        return session._batches[0].redactions[0]

    def test_the_erasure_starts_at_the_edited_span_not_the_line(self):
        doc, line = self._mixed_line()
        partial = self._redaction_for(doc, line, "left", 1)
        assert partial.x0 == pytest.approx(line["spans"][1]["bbox"][0], abs=_REDACT_PAD + 0.1)
        assert partial.x1 == pytest.approx(line["bbox"][2], abs=_REDACT_PAD + 0.1)
        doc.close()

    def test_editing_the_first_span_erases_the_whole_line(self):
        doc, line = self._mixed_line()
        whole = self._redaction_for(doc, line, "left", 0)
        assert whole.x0 == pytest.approx(line["bbox"][0], abs=_REDACT_PAD + 0.1)
        doc.close()

    def test_a_centred_line_is_erased_whole(self):
        """Centring moves every span when the width changes, so none can stay."""
        doc, line = self._mixed_line()
        for align in ("center", "right"):
            rect = self._redaction_for(doc, line, align, 1)
            assert rect.x0 == pytest.approx(line["bbox"][0], abs=_REDACT_PAD + 0.1), align
        doc.close()

    def test_the_untouched_label_survives_the_edit(self):
        doc, line = self._mixed_line()
        before = dict(line["spans"][0])
        spans = [dict(span) for span in line["spans"]]
        spans[1]["text"] = "valor mucho mas largo que antes"
        apply_operations(doc, FontResolver(doc), [{
            "op": "replace_line", "page": 0, "bbox": line["bbox"], "origin": line["origin"],
            "rotation": line["rotation"], "align": "left", "from_span": 1, "spans": spans,
        }])
        after = extract_page(doc, 0, FontResolver(doc))
        kept = next(
            span
            for item in after["lines"]
            for span in item["spans"]
            if span["text"].startswith("Etiqueta")
        )
        assert kept["bbox"] == pytest.approx(before["bbox"], abs=0.1)
        assert kept["font"] == before["font"]
        doc.close()

    def test_the_edited_span_still_changes(self):
        doc, line = self._mixed_line()
        spans = [dict(span) for span in line["spans"]]
        spans[1]["text"] = "TEXTO SUSTITUIDO"
        apply_operations(doc, FontResolver(doc), [{
            "op": "replace_line", "page": 0, "bbox": line["bbox"], "origin": line["origin"],
            "rotation": line["rotation"], "align": "left", "from_span": 1, "spans": spans,
        }])
        assert "TEXTO SUSTITUIDO" in text_of(doc)
        assert "valor que si se edita" not in text_of(doc)
        doc.close()

    def test_an_out_of_range_index_is_ignored(self):
        doc, line = self._mixed_line()
        rect = self._redaction_for(doc, line, "left", 99)
        assert rect.x0 == pytest.approx(line["bbox"][0], abs=_REDACT_PAD + 0.1)
        doc.close()


@requires_fonts
class TestParagraphReflow:
    """Editing running text used to leave one line overflowing into the margin,
    because nothing pulled the extra words down into the lines below."""

    def _paragraph(self, width_words=9, lines=4, follower_gap=None):
        """A block of running text, wide enough to have a real measure.

        ``follower_gap`` puts another paragraph that many points below it, so a
        paragraph that grows has something real to collide with.
        """
        rows, y = [], 100
        for index in range(lines):
            text = " ".join(f"palabra{index}{n}" for n in range(width_words))
            rows.append(((72, y), text, "serif", 11))
            y += 14
        if follower_gap is not None:
            # Set differently on purpose: otherwise it reads as one more line of
            # the same paragraph and gets swept into it.
            rows.append(((72, y + follower_gap), "LO QUE VIENE DEBAJO", "serif-bold", 11))
        doc = pymupdf.open(stream=build_pdf(rows), filetype="pdf")
        page = extract_page(doc, 0, FontResolver(doc))
        group = [line for line in page["lines"] if line["paragraph_size"] > 1]
        assert len(group) >= 3, "el PDF de prueba no produjo un párrafo"
        return doc, group

    def _follower_top(self, doc):
        """Where the paragraph below starts — the ceiling the text must respect."""
        page = extract_page(doc, 0, FontResolver(doc))
        return next(l["bbox"][1] for l in page["lines"] if "DEBAJO" in l["text"])

    def _reflow(self, doc, group, edited_line, new_text, **extra):
        payload = [dict(line) for line in group]
        payload[edited_line] = dict(payload[edited_line])
        payload[edited_line]["spans"] = [dict(s) for s in group[edited_line]["spans"]]
        payload[edited_line]["spans"][0]["text"] = new_text
        return apply_operations(doc, FontResolver(doc), [{
            "op": "replace_paragraph", "page": 0, "box": group[0]["block_bbox"],
            "align": group[0]["align"], "lines": payload,
            "edited": {"line": edited_line, "span": 0}, **extra,
        }])

    def test_a_line_that_grows_pushes_words_down_instead_of_overflowing(self):
        doc, group = self._paragraph()
        margin = group[0]["block_bbox"][2]
        self._reflow(doc, group, 0, group[0]["text"] + " " + " ".join(["añadida"] * 12))
        after = extract_page(doc, 0, FontResolver(doc))
        for line in after["lines"]:
            assert line["bbox"][2] <= margin + 1, f"se sale del margen: {line['text']!r}"
        doc.close()

    def test_no_word_is_lost_when_re_wrapping(self):
        doc, group = self._paragraph()
        before = set(" ".join(line["text"] for line in group).split())
        self._reflow(doc, group, 1, group[1]["text"] + " intercalada")
        after = set(text_of(doc).split())
        assert before <= after, f"se perdieron {before - after}"
        assert "intercalada" in after
        doc.close()

    def test_re_wrapping_unchanged_text_reproduces_the_same_lines(self):
        """The invariant the whole feature rests on. If re-breaking a paragraph
        nobody edited moves its line endings, then every edit quietly reflows
        text around it — which is what a mis-measured stand-in font used to do."""
        doc, group = self._paragraph()
        before = [line["text"] for line in group]
        payload = [dict(line) for line in group]
        apply_operations(doc, FontResolver(doc), [{
            "op": "replace_paragraph", "page": 0, "box": group[0]["block_bbox"],
            "align": group[0]["align"], "lines": payload,
            "edited": {"line": 0, "span": 0}, "reflow": True,
        }])
        after = extract_page(doc, 0, FontResolver(doc))
        rows = [l["text"] for l in after["lines"] if "palabra" in l["text"]]
        assert rows == before, "los saltos de línea cambiaron sin tocar el texto"
        doc.close()

    def test_growth_into_free_space_is_not_reported(self):
        """Nothing is below, so more lines are not a problem worth a warning."""
        doc, group = self._paragraph()
        warnings = self._reflow(doc, group, 0, group[0]["text"] + " añadido corto")
        assert not any(w.kind == "paragraph-grew" for w in warnings)
        doc.close()

    def test_growth_into_the_next_paragraph_is_reported(self):
        doc, group = self._paragraph(follower_gap=8)
        warnings = self._reflow(doc, group, 0, group[0]["text"] + " " + " ".join(["mas"] * 60))
        assert any(w.kind == "paragraph-grew" for w in warnings)
        doc.close()

    def test_a_little_growth_is_absorbed_by_tightening_the_spacing(self):
        """Losing some line spacing is far less visible than resizing the text."""
        doc, group = self._paragraph(follower_gap=8)
        sizes_before = {round(s["size"], 1) for line in group for s in line["spans"]}
        self._reflow(doc, group, 0, group[0]["text"] + " unaPalabraMas")
        after = extract_page(doc, 0, FontResolver(doc))
        rows = [l for l in after["lines"] if "palabra" in l["text"]]
        assert len(rows) > len(group), "debería haber ganado una línea"
        assert {round(s["size"], 1) for l in rows for s in l["spans"]} == sizes_before
        doc.close()

    def test_shrink_keeps_the_paragraph_inside_the_room_it_had(self):
        doc, group = self._paragraph(follower_gap=8)
        ceiling = self._follower_top(doc)
        self._reflow(doc, group, 0, group[0]["text"] + " " + " ".join(["extra"] * 14),
                     fit="shrink")
        after = extract_page(doc, 0, FontResolver(doc))
        rewritten = [l for l in after["lines"] if "palabra" in l["text"]]
        assert max(l["bbox"][3] for l in rewritten) <= ceiling + 1, "invade lo de debajo"
        doc.close()

    def test_the_lines_do_not_run_into_each_other_when_the_size_grows(self):
        """Keeping the original leading after a size increase overlaps them."""
        doc, group = self._paragraph()
        payload = [dict(line) for line in group]
        payload[0] = dict(payload[0])
        payload[0]["spans"] = [dict(s) for s in group[0]["spans"]]
        payload[0]["spans"][0]["size"] = 24
        apply_operations(doc, FontResolver(doc), [{
            "op": "replace_paragraph", "page": 0, "box": group[0]["block_bbox"],
            "align": group[0]["align"], "lines": payload,
            "edited": {"line": 0, "span": 0}, "reflow": True,
        }])
        after = extract_page(doc, 0, FontResolver(doc))
        rows = sorted(
            (l for l in after["lines"] if "palabra" in l["text"]),
            key=lambda item: item["bbox"][1],
        )
        for previous, current in zip(rows, rows[1:]):
            assert current["bbox"][1] >= previous["bbox"][3] - 0.5, "las líneas se solapan"
        doc.close()

    def test_an_edit_that_still_fits_does_not_disturb_the_paragraph(self):
        """Only the edited line is touched while it stays inside the margin."""
        doc, group = self._paragraph()
        before = [list(line["bbox"]) for line in group[1:]]
        self._reflow(doc, group, 0, "corta")
        after = extract_page(doc, 0, FontResolver(doc))
        rows = [l for l in after["lines"] if "palabra1" in l["text"] or "palabra2" in l["text"]]
        for line in rows:
            assert any(
                line["bbox"][1] == pytest.approx(box[1], abs=0.2) for box in before
            ), "una línea que no se editó se movió"
        doc.close()

    def test_a_single_line_paragraph_is_never_reflowed(self, doc, resolver):
        page = extract_page(doc, 0, resolver)
        alone = next(line for line in page["lines"] if line["paragraph_size"] == 1)
        spans = [dict(s) for s in alone["spans"]]
        spans[0]["text"] = "texto sustituido"
        apply_operations(doc, resolver, [{
            "op": "replace_paragraph", "page": 0, "box": alone["block_bbox"],
            "align": alone["align"], "lines": [{**alone, "spans": spans}],
            "edited": {"line": 0, "span": 0},
        }])
        assert "texto sustituido" in text_of(doc)

    def test_links_follow_the_words_they_sat_over(self, linked_doc):
        """Re-wrapping moves every word; the links have to move with them."""
        page = extract_page(linked_doc, 0, FontResolver(linked_doc))
        group = [line for line in page["lines"] if line["paragraph_size"] > 1]
        assert len(group) >= 2, "el PDF enlazado no produjo un párrafo de varias líneas"
        before = {link["uri"] for link in linked_doc[0].get_links()}
        payload = [dict(line) for line in group]
        payload[0]["spans"] = [dict(s) for s in group[0]["spans"]]
        payload[0]["spans"][0]["text"] += " con bastante texto adicional para forzar el corte"
        apply_operations(linked_doc, FontResolver(linked_doc), [{
            "op": "replace_paragraph", "page": 0, "box": group[0]["block_bbox"],
            "align": group[0]["align"], "lines": payload,
            "edited": {"line": 0, "span": 0}, "reflow": True,
        }])
        after = {link["uri"] for link in linked_doc[0].get_links()}
        assert before <= after, f"se perdieron {before - after}"


@requires_fonts
class TestReflowOfRotatedText:
    """Text does not always read left to right. Re-wrapping used to be refused
    outright for anything at a quarter turn."""

    @pytest.mark.parametrize("rotation", [0, 90, 180, 270])
    def test_a_rotated_paragraph_re_wraps(self, rotation):
        from tests.conftest import build_rotated_paragraph

        doc = pymupdf.open(stream=build_rotated_paragraph(rotation), filetype="pdf")
        page = extract_page(doc, 0, FontResolver(doc))
        group = [line for line in page["lines"] if line["paragraph_size"] > 1]
        assert len(group) >= 2, f"no se agrupó como párrafo a {rotation}°"

        before = set(" ".join(line["text"] for line in group).split())
        payload = [dict(line) for line in group]
        payload[0]["spans"] = [dict(s) for s in group[0]["spans"]]
        payload[0]["spans"][0]["text"] += " AGREGADO AGREGADO AGREGADO"
        apply_operations(doc, FontResolver(doc), [{
            "op": "replace_paragraph", "page": 0, "box": group[0]["block_bbox"],
            "measure_point": group[0]["measure_point"], "align": group[0]["align"],
            "lines": payload, "edited": {"line": 0, "span": 0},
        }])

        after = extract_page(doc, 0, FontResolver(doc))
        rows = [l for l in after["lines"] if "pal" in l["text"] or "AGREGADO" in l["text"]]
        words = set(" ".join(l["text"] for l in rows).split())
        assert before <= words, f"se perdieron {before - words}"
        assert "AGREGADO" in words
        assert {l["rotation"] for l in rows} == {rotation}
        assert len(rows) > len(group), "debería haber ganado una línea"
        doc.close()

    @pytest.mark.parametrize("rotation", [90, 180, 270])
    def test_a_rotated_paragraph_keeps_its_own_measure(self, rotation):
        from app.extract import along_span
        from tests.conftest import build_rotated_paragraph

        doc = pymupdf.open(stream=build_rotated_paragraph(rotation), filetype="pdf")
        page = extract_page(doc, 0, FontResolver(doc))
        group = [line for line in page["lines"] if line["paragraph_size"] > 1]
        limit = group[0]["measure"]
        payload = [dict(line) for line in group]
        payload[0]["spans"] = [dict(s) for s in group[0]["spans"]]
        payload[0]["spans"][0]["text"] += " palabras de mas para forzar el corte"
        apply_operations(doc, FontResolver(doc), [{
            "op": "replace_paragraph", "page": 0, "box": group[0]["block_bbox"],
            "measure_point": group[0]["measure_point"], "align": group[0]["align"],
            "lines": payload, "edited": {"line": 0, "span": 0},
        }])
        after = extract_page(doc, 0, FontResolver(doc))
        for line in after["lines"]:
            if "pal" not in line["text"] and "palabras" not in line["text"]:
                continue
            assert along_span(rotation, line["bbox"])[1] <= limit + 1
        doc.close()


@requires_fonts
class TestPushingContentOutOfTheWay:
    """When a paragraph outgrows its room, `push` moves what is below it
    instead of shrinking the text or giving up."""

    def _setup(self, gap=20.0, top=100.0):
        from tests.conftest import build_pdf_with_section_below

        doc = pymupdf.open(stream=build_pdf_with_section_below(gap, top), filetype="pdf")
        page = extract_page(doc, 0, FontResolver(doc))
        group = [line for line in page["lines"] if line["text"].startswith("pal")]
        group = [line for line in group if line["paragraph"] == group[0]["paragraph"]]
        return doc, group

    def _grow(self, doc, group, words=40, fit="push"):
        payload = [dict(line) for line in group]
        payload[0]["spans"] = [dict(s) for s in group[0]["spans"]]
        payload[0]["spans"][0]["text"] += " " + " ".join(["anadido"] * words)
        return apply_operations(doc, FontResolver(doc), [{
            "op": "replace_paragraph", "page": 0, "box": group[0]["block_bbox"],
            "measure_point": group[0]["measure_point"], "align": group[0]["align"],
            "lines": payload, "edited": {"line": 0, "span": 0}, "fit": fit,
        }])

    def _tops(self, doc):
        page = extract_page(doc, 0, FontResolver(doc))
        return {line["text"]: line["bbox"][1] for line in page["lines"]}

    def test_the_section_below_moves_down(self):
        doc, group = self._setup()
        before = self._tops(doc)["SECCION SIGUIENTE"]
        warnings = self._grow(doc, group)
        after = self._tops(doc)["SECCION SIGUIENTE"]
        assert after > before + 5, "no se movió"
        assert not [w for w in warnings if w.kind in {"paragraph-grew", "cannot-push"}]
        doc.close()

    def test_everything_below_moves_by_the_same_amount(self):
        doc, group = self._setup()
        before = self._tops(doc)
        self._grow(doc, group)
        after = self._tops(doc)
        deltas = {
            round(after[text] - before[text], 1)
            for text in ("SECCION SIGUIENTE", "contenido de la seccion")
        }
        assert len(deltas) == 1, f"se movieron de forma desigual: {deltas}"
        doc.close()

    def test_a_rule_below_moves_with_the_text(self):
        """Leaving the line art behind would be worse than not moving at all."""
        doc, group = self._setup()
        before = [round(d["rect"].y0, 1) for d in doc[0].get_drawings()]
        shift = self._tops(doc)["SECCION SIGUIENTE"]
        self._grow(doc, group)
        shift = self._tops(doc)["SECCION SIGUIENTE"] - shift
        after = [round(d["rect"].y0, 1) for d in doc[0].get_drawings()]
        assert len(after) == len(before), f"cambió el número de trazos: {before} -> {after}"
        assert after[0] == pytest.approx(before[0] + shift, abs=1.0)
        doc.close()

    def test_a_link_below_moves_with_the_text(self):
        doc, group = self._setup()
        before = doc[0].get_links()[0]["from"][1]
        shift = self._tops(doc)["SECCION SIGUIENTE"]
        self._grow(doc, group)
        shift = self._tops(doc)["SECCION SIGUIENTE"] - shift
        links = doc[0].get_links()
        assert len(links) == 1
        assert links[0]["from"][1] == pytest.approx(before + shift, abs=1.0)
        doc.close()

    def test_nothing_is_lost_in_the_move(self):
        doc, group = self._setup()
        before = {t for t in self._tops(doc) if not t.startswith("pal")}
        self._grow(doc, group)
        after = set(self._tops(doc))
        assert before <= after, f"se perdió {before - after}"
        doc.close()

    def test_it_refuses_rather_than_push_content_off_the_page(self):
        """The whole block sits at the foot of the sheet; there is nowhere to go."""
        doc, group = self._setup(top=660)
        warnings = self._grow(doc, group, words=80)
        assert any(w.kind == "cannot-push" for w in warnings)
        doc.close()

    def test_overflow_mode_moves_nothing(self):
        doc, group = self._setup()
        before = self._tops(doc)["SECCION SIGUIENTE"]
        self._grow(doc, group, fit="overflow")
        assert self._tops(doc)["SECCION SIGUIENTE"] == pytest.approx(before, abs=0.2)
        doc.close()


@requires_fonts
class TestMovingFiguresOutOfTheWay:
    """Pushing used to refuse the moment it met anything that was not a line or
    a rectangle. Half a figure left in its old place is worse than a paragraph
    that overflows — but moving it is better than either."""

    def _setup(self, top=100.0):
        from tests.conftest import build_pdf_with_figures

        doc = pymupdf.open(stream=build_pdf_with_figures(top), filetype="pdf")
        page = extract_page(doc, 0, FontResolver(doc))
        group = [line for line in page["lines"] if line["text"].startswith("pal")]
        group = [line for line in group if line["paragraph"] == group[0]["paragraph"]]
        return doc, group

    def _state(self, doc):
        page = doc[0]
        text = {
            "".join(s["text"] for s in line["spans"]): round(line["bbox"][1], 1)
            for block in page.get_text("dict")["blocks"]
            for line in block.get("lines", [])
        }
        return (
            text,
            sorted(round(d["rect"].y0, 1) for d in page.get_drawings()),
            sorted(round(i["bbox"][1], 1) for i in page.get_image_info()),
        )

    def _grow(self, doc, group, words=40, fit="push"):
        payload = [dict(line) for line in group]
        payload[0]["spans"] = [dict(s) for s in group[0]["spans"]]
        payload[0]["spans"][0]["text"] += " " + " ".join(["anadido"] * words)
        return apply_operations(doc, FontResolver(doc), [{
            "op": "replace_paragraph", "page": 0, "box": group[0]["block_bbox"],
            "measure_point": group[0]["measure_point"], "align": group[0]["align"],
            "lines": payload, "edited": {"line": 0, "span": 0}, "fit": fit,
        }])

    def test_a_figure_is_an_obstacle_like_any_other(self):
        """A paragraph used to grow straight over a drawing, because only text
        was looked for below it."""
        doc, group = self._setup()
        warnings = self._grow(doc, group, fit="overflow")
        assert any(w.kind == "paragraph-grew" for w in warnings)
        doc.close()

    def test_curves_quadrilaterals_and_pictures_all_move(self):
        doc, group = self._setup()
        before_text, before_drawings, before_images = self._state(doc)
        self._grow(doc, group)
        after_text, after_drawings, after_images = self._state(doc)

        shift = after_text["texto al final del bloque"] - before_text["texto al final del bloque"]
        assert shift > 5, "el texto de debajo no se movió"
        assert after_drawings == pytest.approx([y + shift for y in before_drawings], abs=1.0)
        assert after_images == pytest.approx([y + shift for y in before_images], abs=1.0)
        doc.close()

    def test_nothing_is_left_behind_or_duplicated(self):
        """Erasing line art only works "if touched"; asking for "if covered"
        leaves the original sitting beside its copy."""
        doc, group = self._setup()
        _text, before_drawings, before_images = self._state(doc)
        self._grow(doc, group)
        _text, after_drawings, after_images = self._state(doc)
        assert len(after_drawings) == len(before_drawings)
        assert len(after_images) == len(before_images)
        doc.close()

    def test_overflow_mode_still_moves_nothing(self):
        doc, group = self._setup()
        _t, before_drawings, before_images = self._state(doc)
        self._grow(doc, group, fit="overflow")
        _t, after_drawings, after_images = self._state(doc)
        assert after_drawings == before_drawings
        assert after_images == before_images
        doc.close()

    def test_it_refuses_rather_than_push_a_figure_off_the_page(self):
        doc, group = self._setup(top=560)
        warnings = self._grow(doc, group, words=80)
        assert any(w.kind == "cannot-push" for w in warnings)
        doc.close()

    def test_a_picture_beside_the_column_is_left_alone(self):
        """It is not in the way; moving it would be moving someone else's page."""
        from tests.conftest import build_pdf

        rows = [
            ((72, 100 + 14 * i), " ".join(f"pal{i}{n}" for n in range(6)), "serif", 11)
            for i in range(3)
        ]
        doc = pymupdf.open(stream=build_pdf(rows), filetype="pdf")
        picture = pymupdf.Pixmap(pymupdf.csRGB, pymupdf.IRect(0, 0, 40, 30))
        picture.set_rect(picture.irect, (30, 90, 160))
        doc[0].insert_image(pymupdf.Rect(430, 160, 520, 220), stream=picture.tobytes("png"))
        doc[0].insert_text((72, 300), "final", fontname="helv", fontsize=11)
        doc = pymupdf.open(stream=doc.tobytes(), filetype="pdf")

        page = extract_page(doc, 0, FontResolver(doc))
        group = [l for l in page["lines"] if l["text"].startswith("pal")]
        group = [l for l in group if l["paragraph"] == group[0]["paragraph"]]
        before = sorted(round(i["bbox"][1], 1) for i in doc[0].get_image_info())
        self._grow(doc, group)
        after = sorted(round(i["bbox"][1], 1) for i in doc[0].get_image_info())
        assert after == before, "movió una imagen que estaba en otra columna"
        doc.close()


@requires_fonts
class TestMovingABlock:
    """Text could be edited where it stood, but not put somewhere else."""

    def _doc(self, rotation=0):
        document = pymupdf.open(stream=build_pdf([
            ((72, 100), "Primera linea del bloque", "serif", 12),
            ((72, 116), "Segunda linea del bloque", "serif", 12),
            ((72, 220), "Texto que no se mueve", "serif", 12),
        ]), filetype="pdf")
        if rotation:
            document[0].set_rotation(rotation)
            document = pymupdf.open(stream=document.tobytes(), filetype="pdf")
        document[0].insert_link({
            "kind": pymupdf.LINK_URI,
            "from": pymupdf.Rect(72, 90, 200, 104),
            "uri": "https://ejemplo.com",
        })
        return pymupdf.open(stream=document.tobytes(), filetype="pdf")

    def _block(self, document):
        page = extract_page(document, 0, FontResolver(document))
        lines = [line for line in page["lines"] if "bloque" in line["text"]]
        return lines, page

    def _move(self, document, lines, dx, dy):
        return apply_operations(document, FontResolver(document), [{
            "op": "move_block", "page": 0, "lines": [dict(line) for line in lines],
            "dx": dx, "dy": dy,
        }])

    def test_the_block_lands_where_it_was_dropped(self):
        document = self._doc()
        lines, _page = self._block(document)
        before = [list(line["bbox"]) for line in lines]
        self._move(document, lines, 140, 200)
        after = self._block(document)[0]
        assert len(after) == len(before)
        for original, moved in zip(before, sorted(after, key=lambda l: l["bbox"][1])):
            assert moved["bbox"][0] == pytest.approx(original[0] + 140, abs=1.5)
            assert moved["bbox"][1] == pytest.approx(original[1] + 200, abs=1.5)
        document.close()

    def test_the_rest_of_the_page_stays_put(self):
        document = self._doc()
        lines, page = self._block(document)
        other = next(l for l in page["lines"] if "no se mueve" in l["text"])
        self._move(document, lines, 100, 150)
        after = extract_page(document, 0, FontResolver(document))
        moved = next(l for l in after["lines"] if "no se mueve" in l["text"])
        assert moved["bbox"] == pytest.approx(other["bbox"], abs=0.2)
        document.close()

    def test_the_text_itself_survives(self):
        document = self._doc()
        lines, _page = self._block(document)
        self._move(document, lines, 60, 120)
        text = text_of(document)
        assert "Primera linea del bloque" in text
        assert "Segunda linea del bloque" in text
        assert "Texto que no se mueve" in text
        document.close()

    def test_a_link_travels_with_the_words_it_sits_on(self):
        document = self._doc()
        lines, _page = self._block(document)
        before = list(document[0].get_links()[0]["from"])
        self._move(document, lines, 90, 130)
        links = document[0].get_links()
        assert len(links) == 1
        assert list(links[0]["from"]) == pytest.approx(
            [before[0] + 90, before[1] + 130, before[2] + 90, before[3] + 130], abs=1.5
        )
        document.close()

    def test_a_click_is_not_a_drag(self):
        document = self._doc()
        lines, _page = self._block(document)
        before = [list(line["bbox"]) for line in lines]
        self._move(document, lines, 0.2, 0.1)
        after = sorted(self._block(document)[0], key=lambda l: l["bbox"][1])
        for original, unmoved in zip(before, after):
            assert unmoved["bbox"] == pytest.approx(original, abs=0.2)
        document.close()

    def test_dropping_it_off_the_page_is_refused(self):
        """Better to say no than to put the text where nobody can read it."""
        document = self._doc()
        lines, _page = self._block(document)
        with pytest.raises(EditError):
            self._move(document, lines, 0, 900)
        document.close()

    def test_nothing_to_move_is_an_error_not_a_crash(self, doc, resolver):
        with pytest.raises(EditError):
            apply_operations(doc, resolver, [{"op": "move_block", "page": 0, "lines": []}])

    @pytest.mark.parametrize("rotation", [90, 270])
    def test_a_drag_on_a_rotated_page_follows_the_cursor(self, rotation):
        """The client drags in the space it sees, not in the page's own."""
        document = self._doc(rotation)
        lines, _page = self._block(document)
        before = sorted(line["bbox"] for line in lines)
        self._move(document, lines, 50, 70)
        after = sorted(line["bbox"] for line in self._block(document)[0])
        assert after[0][0] == pytest.approx(before[0][0] + 50, abs=2.0)
        assert after[0][1] == pytest.approx(before[0][1] + 70, abs=2.0)
        document.close()
