"""Resolving the typefaces a PDF already carries.

Replacing text without replacing its look is the whole point of this editor, so
every edit starts here: find the font program the original characters were drawn
with, check it can actually draw the new characters, and only substitute when it
cannot. A font program lifted out of the PDF is reused byte for byte, which is
what keeps an edited line indistinguishable from its neighbours.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from functools import lru_cache
from typing import Iterable, Sequence

import pymupdf

# Span flag bits as reported by PyMuPDF's text extraction.
FLAG_SUPERSCRIPT = 1
FLAG_ITALIC = 2
FLAG_SERIF = 4
FLAG_MONO = 8
FLAG_BOLD = 16

_SUBSET_PREFIX = re.compile(r"^[A-Z]{6}\+")
_NON_ALNUM = re.compile(r"[^a-z0-9]+")

# Style words that say "this is the plain cut", plus foundry suffixes. They are
# dropped from both sides of a comparison, so the "LiberationSerif" a page
# refers to still finds the program that calls itself "Liberation Serif
# Regular". Weight and slant words are deliberately absent: those distinguish
# real fonts and must never be stripped.
_NEUTRAL_SUFFIX = re.compile(r"(regular|normal|book|roman|std|pro|mt|ps)$")

_BOLD_HINTS = ("bold", "black", "heavy", "semibold", "demibold", "extrabold")
_ITALIC_HINTS = ("italic", "oblique")

# Base-14 fonts, indexed by (serif, mono, bold, italic). These are always
# available in a PDF viewer but only cover Latin-1.
_BASE14 = {
    # sans
    (False, False, False, False): "helv",
    (False, False, True, False): "hebo",
    (False, False, False, True): "heit",
    (False, False, True, True): "hebi",
    # serif
    (True, False, False, False): "tiro",
    (True, False, True, False): "tibo",
    (True, False, False, True): "tiit",
    (True, False, True, True): "tibi",
    # monospace
    (False, True, False, False): "cour",
    (False, True, True, False): "cobo",
    (False, True, False, True): "coit",
    (False, True, True, True): "cobi",
}

# Searched in order when the text needs glyphs no base-14 font has (accents
# beyond Latin-1, Greek, Cyrillic...). Any that is missing is skipped.
_SYSTEM_FONT_DIRS = (
    "/usr/share/fonts/truetype/dejavu",
    "/usr/share/fonts/truetype/liberation",
    "/usr/share/fonts/truetype/freefont",
    "/Library/Fonts",
    "/System/Library/Fonts",
    "C:/Windows/Fonts",
)

_SYSTEM_FONT_FILES = {
    # (serif, mono, bold, italic) -> candidate file names, best first
    (False, False, False, False): ("DejaVuSans.ttf", "LiberationSans-Regular.ttf", "FreeSans.ttf", "Arial.ttf"),
    (False, False, True, False): ("DejaVuSans-Bold.ttf", "LiberationSans-Bold.ttf", "FreeSansBold.ttf"),
    (False, False, False, True): ("DejaVuSans-Oblique.ttf", "LiberationSans-Italic.ttf", "FreeSansOblique.ttf"),
    (False, False, True, True): ("DejaVuSans-BoldOblique.ttf", "LiberationSans-BoldItalic.ttf"),
    (True, False, False, False): ("DejaVuSerif.ttf", "LiberationSerif-Regular.ttf", "FreeSerif.ttf"),
    (True, False, True, False): ("DejaVuSerif-Bold.ttf", "LiberationSerif-Bold.ttf", "FreeSerifBold.ttf"),
    (True, False, False, True): ("DejaVuSerif-Italic.ttf", "LiberationSerif-Italic.ttf", "FreeSerifItalic.ttf"),
    (True, False, True, True): ("DejaVuSerif-BoldItalic.ttf", "LiberationSerif-BoldItalic.ttf"),
    (False, True, False, False): ("DejaVuSansMono.ttf", "LiberationMono-Regular.ttf", "FreeMono.ttf"),
    (False, True, True, False): ("DejaVuSansMono-Bold.ttf", "LiberationMono-Bold.ttf"),
    (False, True, False, True): ("DejaVuSansMono-Oblique.ttf", "LiberationMono-Italic.ttf"),
    (False, True, True, True): ("DejaVuSansMono-BoldOblique.ttf", "LiberationMono-BoldItalic.ttf"),
}


def normalize_font_name(name: str | None) -> str:
    """Reduce a font name to a form that survives PDF naming quirks.

    ``get_fonts()`` reports ``"Liberation Serif Italic"`` for the very font a
    span calls ``"ABCDEF+LiberationSerif-Italic"``, so names are compared with
    the subset prefix, spaces and punctuation stripped.
    """
    stripped = _SUBSET_PREFIX.sub("", name or "")
    return _NON_ALNUM.sub("", stripped.lower())


def canonical_font_name(name: str | None) -> str:
    """A normalized name with neutral style and foundry suffixes removed.

    Matching on this is what lets a span's ``LiberationSerif`` find the embedded
    ``Liberation Serif Regular``; it is only ever compared against another name
    put through the same treatment.
    """
    normalized = normalize_font_name(name)
    trimmed = normalized
    while True:
        shorter = _NEUTRAL_SUFFIX.sub("", trimmed)
        if shorter == trimmed or len(shorter) < 3:
            break
        trimmed = shorter
    return trimmed or normalized


def style_of(font_name: str | None, flags: int = 0) -> tuple[bool, bool, bool, bool]:
    """Return ``(serif, mono, bold, italic)`` for a span.

    The extraction flags are authoritative when set, but plenty of PDFs leave
    them at zero and encode the style in the font name alone, so both are used.
    """
    lowered = (font_name or "").lower()
    bold = bool(flags & FLAG_BOLD) or any(h in lowered for h in _BOLD_HINTS)
    italic = bool(flags & FLAG_ITALIC) or any(h in lowered for h in _ITALIC_HINTS)
    mono = bool(flags & FLAG_MONO) or "mono" in lowered or "courier" in lowered
    serif = bool(flags & FLAG_SERIF) or any(
        h in lowered for h in ("serif", "times", "georgia", "garamond", "roman", "book")
    )
    if mono:
        serif = False
    if "sans" in lowered:
        serif = False
    return serif, mono, bold, italic


@dataclass
class ResolvedFont:
    """A font ready to draw with, plus how faithful it is to the original."""

    key: str
    """Stable identity, used to cache the ``pymupdf.Font`` and page resources."""

    font: pymupdf.Font
    """Loaded font, used to measure text before it is written."""

    buffer: bytes | None = None
    """Embedded font program, when one could be reused."""

    base14: str | None = None
    """Base-14 short name (``helv``...), when no program is embedded."""

    source: str = "embedded"
    """``embedded``, ``embedded-other``, ``system`` or ``base14``."""

    note: str | None = None
    """Why the original font could not be used, when it could not."""

    @property
    def is_faithful(self) -> bool:
        """True when the text is drawn with the exact program it replaced."""
        return self.source == "embedded"

    def text_length(self, text: str, fontsize: float) -> float:
        """Width of ``text`` at ``fontsize``, in points."""
        return self.font.text_length(text, fontsize)

    def missing_glyphs(self, text: str) -> list[str]:
        """Characters of ``text`` this font cannot draw."""
        return sorted({ch for ch in text if ch.strip() and not self.font.has_glyph(ord(ch))})


@lru_cache(maxsize=64)
def _system_font_path(style: tuple[bool, bool, bool, bool]) -> str | None:
    """Locate an installed font file for ``style``, if the machine has one."""
    for candidate in _SYSTEM_FONT_FILES.get(style, ()):  # best match first
        for directory in _SYSTEM_FONT_DIRS:
            path = os.path.join(directory, candidate)
            if os.path.isfile(path):
                return path
    return None


def _covers(font: pymupdf.Font, text: str) -> bool:
    """Whether ``font`` has a glyph for every visible character of ``text``."""
    return all(font.has_glyph(ord(ch)) for ch in text if ch.strip())


@dataclass
class _EmbeddedFont:
    """One font program found in the document."""

    xref: int
    basefont: str
    ext: str
    buffer: bytes
    style: tuple[bool, bool, bool, bool]
    font: pymupdf.Font


class FontResolver:
    """Finds, caches and installs the fonts needed to redraw edited text.

    One resolver serves one open document. Font programs are extracted at most
    once and the ``pymupdf.Font`` objects built from them are shared, because
    parsing a 400 KB font program per edited line is the slowest thing an edit
    would otherwise do.
    """

    def __init__(self, doc: pymupdf.Document) -> None:
        self._doc = doc
        self._embedded: dict[int, _EmbeddedFont] = {}
        self._by_name: dict[str, list[_EmbeddedFont]] = {}
        self._scanned_pages: set[int] = set()
        self._resolved: dict[tuple, ResolvedFont] = {}
        self._installed: dict[tuple[int, str], str] = {}
        self._fallback_cache: dict[tuple, ResolvedFont] = {}

    # -- discovery ---------------------------------------------------------

    def scan_page(self, pno: int) -> None:
        """Extract every embedded font program referenced by one page."""
        if pno in self._scanned_pages:
            return
        self._scanned_pages.add(pno)
        try:
            entries = self._doc[pno].get_fonts(full=False)
        except Exception:
            return
        for entry in entries:
            self._load_embedded(int(entry[0]), str(entry[3]))

    def scan_document(self) -> None:
        """Extract the fonts of every page. Used when matching across pages."""
        for pno in range(self._doc.page_count):
            self.scan_page(pno)

    def _load_embedded(self, xref: int, basefont: str) -> _EmbeddedFont | None:
        if xref in self._embedded:
            return self._embedded[xref]
        try:
            name, ext, _subtype, buffer = self._doc.extract_font(xref)
        except Exception:
            return None
        if not buffer:
            # A base-14 font: the viewer supplies the program, there is nothing
            # to lift out. Still recorded by name so lookups do not rescan.
            self._embedded[xref] = None  # type: ignore[assignment]
            return None
        try:
            font = pymupdf.Font(fontbuffer=buffer)
        except Exception:
            return None
        record = _EmbeddedFont(
            xref=xref,
            basefont=name or basefont,
            ext=ext or "",
            buffer=buffer,
            style=style_of(name or basefont),
            font=font,
        )
        self._embedded[xref] = record
        for source in (name, basefont):
            for key in {normalize_font_name(source), canonical_font_name(source)}:
                if key and record not in self._by_name.setdefault(key, []):
                    self._by_name[key].append(record)
        return record

    # -- resolution --------------------------------------------------------

    def resolve(
        self,
        pno: int,
        font_name: str | None,
        flags: int = 0,
        text: str = "",
        *,
        allow_substitution: bool = True,
    ) -> ResolvedFont:
        """Pick the best font to redraw ``text`` that was set in ``font_name``.

        The search widens only as far as it has to: the exact embedded program
        first, then another embedded program of the same style that covers the
        text, then an installed system font, then base-14.
        """
        self.scan_page(pno)
        cache_key = (font_name or "", flags, "".join(sorted(set(text))), allow_substitution)
        cached = self._resolved.get(cache_key)
        if cached is not None:
            return cached

        resolved = self._resolve_uncached(font_name, flags, text, allow_substitution)
        self._resolved[cache_key] = resolved
        return resolved

    def _candidates(self, font_name: str | None) -> list[_EmbeddedFont]:
        """Embedded programs that could be the one ``font_name`` refers to."""
        for key in (normalize_font_name(font_name), canonical_font_name(font_name)):
            found = self._by_name.get(key)
            if found:
                return found
        return []

    def _resolve_uncached(
        self, font_name: str | None, flags: int, text: str, allow_substitution: bool
    ) -> ResolvedFont:
        style = style_of(font_name, flags)

        # 1. The program this very text was drawn with.
        for record in self._candidates(font_name):
            if not allow_substitution or _covers(record.font, text):
                return ResolvedFont(
                    key=f"emb:{record.xref}",
                    font=record.font,
                    buffer=record.buffer,
                    source="embedded",
                )
        exact_exists = bool(self._candidates(font_name))

        if not allow_substitution and not exact_exists:
            # Caller insists on the original; fall through to the normal chain
            # since there is no original to insist on.
            pass

        note = (
            f"«{font_name}» no tiene glifos para algunos caracteres nuevos"
            if exact_exists
            else f"«{font_name}» no está embebida en el PDF"
        )

        # 2. Another program already in the document with the same style.
        for record in self._document_fonts_by_style(style):
            if _covers(record.font, text):
                return ResolvedFont(
                    key=f"emb:{record.xref}",
                    font=record.font,
                    buffer=record.buffer,
                    source="embedded-other",
                    note=note,
                )

        return self._fallback(style, text, note)

    def _document_fonts_by_style(self, style: tuple[bool, bool, bool, bool]) -> list[_EmbeddedFont]:
        """Embedded fonts of the document, closest in style first."""
        records = [r for r in self._embedded.values() if r is not None]
        return sorted(records, key=lambda r: sum(a != b for a, b in zip(r.style, style)))

    def _fallback(
        self, style: tuple[bool, bool, bool, bool], text: str, note: str | None
    ) -> ResolvedFont:
        """A system or base-14 font, whichever first covers ``text``."""
        cache_key = (style, "".join(sorted(set(text))))
        cached = self._fallback_cache.get(cache_key)
        if cached is not None:
            return ResolvedFont(
                key=cached.key,
                font=cached.font,
                buffer=cached.buffer,
                base14=cached.base14,
                source=cached.source,
                note=note or cached.note,
            )

        base14 = _BASE14.get(style, "helv")
        path = _system_font_path(style)
        if path:
            try:
                with open(path, "rb") as handle:
                    buffer = handle.read()
                font = pymupdf.Font(fontbuffer=buffer)
                if _covers(font, text):
                    resolved = ResolvedFont(
                        key=f"sys:{path}",
                        font=font,
                        buffer=buffer,
                        source="system",
                        note=note,
                    )
                    self._fallback_cache[cache_key] = resolved
                    return resolved
            except Exception:
                pass

        resolved = ResolvedFont(
            key=f"b14:{base14}",
            font=pymupdf.Font(fontname=base14),
            base14=base14,
            source="base14",
            note=note,
        )
        self._fallback_cache[cache_key] = resolved
        return resolved

    def resolve_family(
        self,
        pno: int,
        family: str,
        *,
        bold: bool = False,
        italic: bool = False,
        text: str = "",
    ) -> ResolvedFont:
        """Resolve a font the user picked by name in the formatting toolbar.

        ``family`` may name a font the document already embeds (so a new text
        box can match the document) or one of the generic families.
        """
        self.scan_document()
        generic = {
            "sans": (False, False, bold, italic),
            "serif": (True, False, bold, italic),
            "mono": (False, True, bold, italic),
        }
        if family in generic:
            return self._fallback(generic[family], text, None)

        style = style_of(family, (FLAG_BOLD if bold else 0) | (FLAG_ITALIC if italic else 0))
        candidates = self._candidates(family)
        if candidates:
            # Prefer a weight/slant variant of the same family when embedded.
            best = min(candidates, key=lambda r: sum(a != b for a, b in zip(r.style, style)))
            # Asking for bold or italic must actually produce bold or italic: an
            # embedded regular weight is the wrong answer to that request, so it
            # is passed over for a system font that has the style.
            style_matches = best.style[2] == bold and best.style[3] == italic
            if style_matches and _covers(best.font, text):
                return ResolvedFont(key=f"emb:{best.xref}", font=best.font, buffer=best.buffer, source="embedded")
        return self._fallback(style, text, None)

    def available_families(self) -> list[dict]:
        """Fonts offered in the toolbar: the document's own, then generics."""
        self.scan_document()
        seen: dict[str, dict] = {}
        for record in self._embedded.values():
            if record is None:
                continue
            name = _SUBSET_PREFIX.sub("", record.basefont)
            if name not in seen:
                serif, mono, bold, italic = record.style
                seen[name] = {
                    "name": name,
                    "source": "document",
                    "bold": bold,
                    "italic": italic,
                    "mono": mono,
                    "serif": serif,
                }
        families = sorted(seen.values(), key=lambda item: item["name"].lower())
        for generic in ("sans", "serif", "mono"):
            families.append(
                {
                    "name": generic,
                    "source": "generic",
                    "bold": False,
                    "italic": False,
                    "mono": generic == "mono",
                    "serif": generic == "serif",
                }
            )
        return families

    # -- installation ------------------------------------------------------

    def install(self, page: pymupdf.Page, resolved: ResolvedFont) -> str:
        """Add ``resolved`` to ``page``'s resources and return its draw name.

        A page keeps one resource per distinct font, so repeated edits on the
        same page with the same typeface do not grow the file.
        """
        if resolved.base14 and resolved.buffer is None:
            return resolved.base14
        cache_key = (page.number, resolved.key)
        existing = self._installed.get(cache_key)
        if existing:
            return existing
        name = f"PE{abs(hash(resolved.key)) % 10**8:08d}"
        try:
            page.insert_font(fontname=name, fontbuffer=resolved.buffer)
        except Exception:
            # A program MuPDF can measure but not embed; fall back to base-14.
            return resolved.base14 or "helv"
        self._installed[cache_key] = name
        return name

    def forget_page(self, pno: int) -> None:
        """Drop cached page resources after the page content was rewritten."""
        for key in [k for k in self._installed if k[0] == pno]:
            del self._installed[key]
