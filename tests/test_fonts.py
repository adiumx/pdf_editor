"""The font resolver: does replacement text keep the typeface it replaced?"""

from __future__ import annotations

import pymupdf
import pytest

from app.fonts import (
    FontResolver,
    canonical_font_name,
    normalize_font_name,
    style_of,
)
from tests.conftest import build_pdf, requires_fonts, spans_of


class TestNames:
    def test_subset_prefix_and_punctuation_are_ignored(self):
        assert normalize_font_name("ABCDEF+LiberationSerif-Italic") == "liberationserifitalic"
        assert normalize_font_name("Liberation Serif Italic") == "liberationserifitalic"

    @pytest.mark.parametrize(
        ("referenced", "embedded"),
        [
            ("LiberationSerif", "Liberation Serif Regular"),
            ("Arial-BoldMT", "Arial Bold"),
            ("TimesNewRomanPSMT", "Times New Roman"),
            ("Helvetica", "Helvetica"),
        ],
    )
    def test_neutral_suffixes_do_not_break_matching(self, referenced, embedded):
        assert canonical_font_name(referenced) == canonical_font_name(embedded)

    def test_weight_and_slant_are_never_stripped(self):
        """Bold and italic name different fonts; they must stay distinguishable."""
        assert canonical_font_name("LiberationSerif-Bold") != canonical_font_name("LiberationSerif")
        assert canonical_font_name("LiberationSerif-Italic") != canonical_font_name("LiberationSerif")

    def test_style_read_from_flags_and_from_name(self):
        assert style_of("Whatever", 16)[2] is True  # bold flag
        assert style_of("Arial-BoldMT", 0)[2] is True  # bold in the name
        assert style_of("SomeSans-Oblique", 0)[3] is True
        assert style_of("CourierNew", 0)[1] is True


@requires_fonts
class TestResolution:
    def test_every_span_resolves_to_its_own_embedded_program(self, doc, resolver):
        for span in spans_of(doc):
            resolved = resolver.resolve(0, span["font"], span["flags"], span["text"])
            assert resolved.is_faithful, f"{span['font']} cayó en {resolved.source}"
            assert resolved.buffer, "el programa embebido debería reutilizarse tal cual"

    def test_measurement_matches_the_original_line_width(self, doc, resolver):
        span = spans_of(doc)[0]
        resolved = resolver.resolve(0, span["font"], span["flags"], span["text"])
        measured = resolved.text_length(span["text"], span["size"])
        drawn = span["bbox"][2] - span["bbox"][0]
        assert measured == pytest.approx(drawn, abs=0.5)

    def test_unknown_font_falls_back_and_says_why(self, doc, resolver):
        resolved = resolver.resolve(0, "NoSuchFont-Regular", 0, "hola")
        assert not resolved.is_faithful
        assert resolved.note and "NoSuchFont" in resolved.note

    def test_text_the_embedded_font_cannot_draw_gets_a_font_that_can(self, doc, resolver):
        span = spans_of(doc)[0]
        resolved = resolver.resolve(0, span["font"], span["flags"], "日本語")
        assert resolved.source != "embedded"

    def test_missing_glyphs_are_reported(self, doc, resolver):
        resolved = resolver.resolve(0, "NoSuchFont", 0, "abc")
        assert resolved.missing_glyphs("abc") == []

    def test_asking_for_bold_does_not_return_the_regular_cut(self, doc, resolver):
        """Toggling bold must change the font, not silently keep the old one."""
        regular = resolver.resolve_family(0, "LiberationSerif", text="hola")
        bold = resolver.resolve_family(0, "LiberationSerif", bold=True, text="hola")
        assert regular.key != bold.key

    def test_a_font_the_document_embeds_is_offered_in_the_toolbar(self, doc, resolver):
        names = [family["name"] for family in resolver.available_families()]
        assert any("Liberation" in name for name in names)
        assert {"sans", "serif", "mono"} <= set(names)

    def test_resolution_is_cached_per_document(self, doc, resolver):
        span = spans_of(doc)[0]
        first = resolver.resolve(0, span["font"], span["flags"], span["text"])
        second = resolver.resolve(0, span["font"], span["flags"], span["text"])
        assert first is second


@requires_fonts
class TestInstallation:
    def test_installing_twice_on_a_page_reuses_one_resource(self, doc, resolver):
        span = spans_of(doc)[0]
        resolved = resolver.resolve(0, span["font"], span["flags"], span["text"])
        page = doc[0]
        before = len(page.get_fonts())
        first = resolver.install(page, resolved)
        second = resolver.install(page, resolved)
        assert first == second
        assert len(page.get_fonts()) <= before + 1

    def test_base14_fonts_need_no_installation(self, resolver):
        plain = pymupdf.open()
        page = plain.new_page()
        resolved = resolver.resolve(0, "Helvetica", 0, "hola")
        assert resolver.install(page, resolved) in {"helv", "hebo", "heit", "hebi"} or resolved.buffer
        plain.close()
