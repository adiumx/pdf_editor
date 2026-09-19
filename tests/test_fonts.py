"""The font resolver: does replacement text keep the typeface it replaced?"""

from __future__ import annotations

import pymupdf
import pytest

from app.fonts import (
    FLAG_BOLD,
    FontResolver,
    _strip_style_words,
    canonical_font_name,
    find_match,
    find_system_font,
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


@requires_fonts
class TestFontsTheDocumentNamesButDoesNotEmbed:
    """A PDF may name a font without carrying it. Redrawing such text used to
    pick a stand-in by style alone, which turned Arial into DejaVu Sans and
    changed how an edited line looked next to its neighbours."""

    def test_the_named_font_is_used_when_the_machine_has_it(self, unembedded_doc):
        resolver = FontResolver(unembedded_doc)
        resolved = resolver.resolve(0, "DejaVuSerif-Bold", 0, "hola")
        assert resolved.source == "system-named"
        assert "DejaVuSerif-Bold" in resolved.key
        assert resolved.is_faithful, "no debería marcarse como sustitución"

    def test_a_named_font_produces_no_substitution_warning(self, unembedded_doc):
        resolver = FontResolver(unembedded_doc)
        assert resolver.resolve(0, "DejaVuSerif", 0, "hola").note is None

    def test_arial_resolves_to_a_metric_compatible_sans_not_any_serif(self, unembedded_doc):
        """Arial is almost never embedded; it must not become a serif face."""
        resolver = FontResolver(unembedded_doc)
        resolved = resolver.resolve(0, "Arial-BoldMT", FLAG_BOLD, "hola")
        assert "LiberationSans" in resolved.key
        assert resolved.source == "system-named"

    @pytest.mark.parametrize(
        ("named", "expected"),
        [
            ("TimesNewRomanPSMT", "LiberationSerif"),
            ("CourierNew", "LiberationMono"),
            ("Calibri", "LiberationSans"),
            ("Georgia-Italic", "LiberationSerif-Italic"),
        ],
    )
    def test_common_unembedded_fonts_map_to_their_free_equivalents(self, named, expected):
        found = find_system_font(named)
        assert found is not None, f"{named} no encontró equivalente"
        assert expected in found.path

    def test_a_font_nobody_has_still_falls_back_and_warns(self, unembedded_doc):
        resolver = FontResolver(unembedded_doc)
        resolved = resolver.resolve(0, "FuenteQueNoExisteEnNingunSitio", 0, "hola")
        assert not resolved.is_faithful
        assert resolved.note

    def test_the_toolbar_offers_the_machines_fonts_not_just_three_generics(self, unembedded_doc):
        """With nothing embedded, the picker used to show only sans/serif/mono."""
        families = FontResolver(unembedded_doc).available_families()
        system = [family for family in families if family["source"] == "system"]
        assert len(system) >= 3
        assert len(families) > 3

    def test_a_family_picked_from_the_toolbar_is_honoured_by_name(self, unembedded_doc):
        resolver = FontResolver(unembedded_doc)
        serif = resolver.resolve_family(0, "DejaVu Serif", text="hola")
        mono = resolver.resolve_family(0, "Liberation Mono", text="hola")
        assert "DejaVuSerif" in serif.key
        assert "LiberationMono" in mono.key

    def test_bold_and_italic_pick_the_right_cut_of_the_chosen_family(self, unembedded_doc):
        resolver = FontResolver(unembedded_doc)
        plain = resolver.resolve_family(0, "DejaVu Serif", text="hola")
        bold = resolver.resolve_family(0, "DejaVu Serif", bold=True, text="hola")
        italic = resolver.resolve_family(0, "DejaVu Serif", italic=True, text="hola")
        assert "Bold" in bold.key and "Bold" not in plain.key
        assert "Italic" in italic.key

    def test_family_names_are_read_from_the_font_not_guessed_from_the_file(self):
        """`c0419bt_.pfb` calls itself "Courier 10 Pitch"; the file name is junk."""
        found = find_system_font("Courier 10 Pitch")
        assert found is not None
        assert found.family == "Courier 10 Pitch"

    def test_the_family_name_drops_the_cut(self):
        assert _strip_style_words("DejaVu Serif Bold Italic") == "DejaVu Serif"
        assert _strip_style_words("Courier 10 Pitch Bold") == "Courier 10 Pitch"
        assert _strip_style_words("Unifont-JP Regular") == "Unifont-JP"

    def test_editing_such_a_line_keeps_the_typeface(self, unembedded_doc):
        """End to end: the visible complaint was the font changing on apply."""
        from app.editor import apply_operations
        from app.extract import extract_page

        resolver = FontResolver(unembedded_doc)
        page = extract_page(unembedded_doc, 0, resolver)
        line = next(l for l in page["lines"] if "DejaVuSerif-Bold" in l["text"])
        assert line["spans"][0]["embedded"] is True, "no debería marcarse como problemática"

        spans = [dict(span) for span in line["spans"]]
        spans[0]["text"] = "Texto reescrito"
        warnings = apply_operations(unembedded_doc, resolver, [{
            "op": "replace_line", "page": 0, "bbox": line["bbox"], "origin": line["origin"],
            "rotation": line["rotation"], "align": line["align"], "spans": spans,
        }])
        assert not [w for w in warnings if w.kind == "font-substituted"]

        after = extract_page(unembedded_doc, 0, FontResolver(unembedded_doc))
        written = next(l for l in after["lines"] if "Texto reescrito" in l["text"])
        assert "DejaVuSerif" in written["spans"][0]["font"].replace(" ", "")

    def test_a_requested_style_the_family_lacks_is_honoured_elsewhere(self, unembedded_doc):
        """This machine has no DejaVu Serif italic. Asking for italic must not
        quietly return upright text — the cut matters more than the family."""
        match = find_match("DejaVu Serif", (True, False, False, True))
        assert match is not None
        assert match.font.style[3] is True, "el resultado debería ser cursivo"
        assert match.kind == "cut", "y debería constar que se cambió de familia"

    def test_changing_family_to_honour_a_style_is_reported(self, unembedded_doc):
        resolved = FontResolver(unembedded_doc).resolve_family(
            0, "DejaVu Serif", italic=True, text="hola"
        )
        assert resolved.note and "cursiva" in resolved.note


@requires_fonts
class TestCommonFontsAndFallbacks:
    def test_the_style_fallback_finds_a_real_file(self):
        """It used to join a font's name onto directory roots like
        /usr/share/fonts, where no font actually sits, so every fallback that
        got this far ended up on a base-14 font instead of an installed one."""
        import os

        from app.fonts import _system_font_path

        for style in [
            (False, False, False, False),
            (True, False, True, False),
            (False, True, False, False),
        ]:
            path = _system_font_path(style)
            assert path is not None, f"sin fuente para {style}"
            assert os.path.isfile(path), path

    @pytest.mark.parametrize(
        "named",
        ["Carlito-Bold", "Calibri", "Roboto", "Consolas", "Montserrat", "Palatino",
         "OpenSans", "Helvetica Neue", "Trebuchet MS", "Merriweather"],
    )
    def test_common_font_names_all_resolve_to_something_installed(self, named):
        """These are named by documents constantly and embedded almost never."""
        assert find_system_font(named) is not None

    def test_a_font_the_pdf_names_but_cannot_be_used_does_not_fall_to_base14(self):
        """Carlito is subsetted without a usable character map in real CVs, so
        the embedded program cannot draw new text. The answer is Carlito, or its
        metric-compatible stand-in — not Helvetica."""
        from tests.conftest import build_pdf_naming_fonts_it_does_not_embed

        document = pymupdf.open(
            stream=build_pdf_naming_fonts_it_does_not_embed({"F1": "Carlito-Bold"}),
            filetype="pdf",
        )
        resolved = FontResolver(document).resolve(0, "Carlito-Bold", FLAG_BOLD, "Hola")
        assert resolved.source != "base14"
        assert resolved.buffer, "debería usar un programa de fuente real"
        document.close()

    def test_the_standard_pdf_fonts_are_offered_by_name(self, doc, resolver):
        """They need no installation and every viewer has them."""
        families = resolver.available_families()
        standard = {family["name"] for family in families if family["source"] == "standard"}
        assert {"Helvetica", "Times", "Courier"} <= standard

    @pytest.mark.parametrize(
        ("family", "expected"),
        [("Helvetica", "helv"), ("Times", "tiro"), ("Courier", "cour")],
    )
    def test_a_standard_font_stays_a_standard_font(self, doc, resolver, family, expected):
        """Picking one must not embed a look-alike; that is the point of them."""
        resolved = resolver.resolve_family(0, family, text="hola")
        assert resolved.base14 == expected
        assert resolved.buffer is None

    def test_a_standard_font_honours_bold_and_italic(self, doc, resolver):
        assert resolver.resolve_family(0, "Helvetica", bold=True, text="x").base14 == "hebo"
        assert resolver.resolve_family(0, "Times", italic=True, text="x").base14 == "tiit"
