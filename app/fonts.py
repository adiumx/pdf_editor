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

# Where installed fonts live. Scanned once, so a PDF that names a font without
# embedding it can still be redrawn with that very font when the machine has it.
_SYSTEM_FONT_DIRS = (
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    os.path.expanduser("~/.fonts"),
    os.path.expanduser("~/.local/share/fonts"),
    "/Library/Fonts",
    "/System/Library/Fonts",
    os.path.expanduser("~/Library/Fonts"),
    "C:/Windows/Fonts",
)

# Scanning stops here; no realistic machine needs more, and a runaway directory
# should not stall the first edit.
_MAX_SYSTEM_FONTS = 4000

# How many font programs are opened to read their real name. A name read from
# the font itself is far better than one guessed from its file name — the file
# `c0419bt_.pfb` calls itself "Courier 10 Pitch" — but each one costs a parse,
# so past this many the file name has to do.
_MAX_NAMED_FONTS = 400

_FONT_EXTENSIONS = (".ttf", ".otf", ".ttc", ".pfb")

# Fonts a PDF commonly names but rarely embeds, and the metric-compatible free
# families that stand in for them. Without this, a document set in Arial would
# be redrawn in whatever sans happened to be listed first.
_METRIC_ALIASES = {
    "arial": ("liberationsans", "dejavusans", "freesans"),
    "helvetica": ("liberationsans", "dejavusans", "freesans"),
    "arialnarrow": ("liberationsansnarrow", "liberationsans"),
    "timesnewroman": ("liberationserif", "dejavuserif", "freeserif"),
    "times": ("liberationserif", "dejavuserif", "freeserif"),
    "couriernew": ("liberationmono", "dejavusansmono", "freemono"),
    "courier": ("liberationmono", "dejavusansmono", "freemono"),
    "calibri": ("carlito", "liberationsans", "dejavusans"),
    "cambria": ("caladea", "liberationserif", "dejavuserif"),
    "georgia": ("gelasio", "liberationserif", "dejavuserif"),
    "verdana": ("dejavusans", "liberationsans"),
    "tahoma": ("dejavusans", "liberationsans"),
    "garamond": ("ebgaramond", "liberationserif"),
    "segoeui": ("selawik", "dejavusans", "liberationsans"),
}

# Style words stripped to get from a font's own name to its family's name.
_STYLE_WORDS = (
    "bold", "italic", "oblique", "regular", "normal", "book", "roman", "black",
    "heavy", "light", "thin", "medium", "semibold", "demibold", "extrabold",
    "ultrabold", "extralight", "ultralight", "condensed", "narrow", "expanded",
    "std", "pro", "mt", "ps", "web", "cn",
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
    """How the font was found, worst case last:

    ``embedded``      the program the document carries for this very text
    ``system-named``  the font the document *names*, installed on this machine
    ``embedded-other``another program in the document, same style
    ``system``        an installed font of the same style, different name
    ``base14``        one of the fonts every PDF viewer supplies
    """

    note: str | None = None
    """Why the original font could not be used, when it could not."""

    @property
    def is_faithful(self) -> bool:
        """True when the text keeps the typeface the document asked for.

        That covers a font the PDF embeds and one it only names, as long as the
        machine has it: in both cases the replacement is the same typeface, not
        a stand-in.
        """
        return self.source in {"embedded", "system-named"}

    def text_length(self, text: str, fontsize: float) -> float:
        """Width of ``text`` at ``fontsize``, in points."""
        return self.font.text_length(text, fontsize)

    def missing_glyphs(self, text: str) -> list[str]:
        """Characters of ``text`` this font cannot draw."""
        return sorted({ch for ch in text if ch.strip() and not self.font.has_glyph(ord(ch))})


def family_key(name: str | None) -> str:
    """Reduce a font name to its family, dropping every style word.

    ``DejaVuSerif-Bold`` and ``DejaVu Serif`` both come back as ``dejavuserif``,
    which is what lets the toolbar offer one entry per family and apply weight
    and slant on top of it.
    """
    key = normalize_font_name(name)
    while True:
        for word in _STYLE_WORDS:
            if key.endswith(word) and len(key) > len(word) + 2:
                key = key[: -len(word)]
                break
        else:
            return key


def _pretty_family(stem: str) -> str:
    """A human-readable family name from a font file's name."""
    name = _SUBSET_PREFIX.sub("", stem).replace("_", " ").replace("-", " ")
    spaced = re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", name)
    words = [
        word
        for word in spaced.split()
        if normalize_font_name(word) not in _STYLE_WORDS or not word
    ]
    return " ".join(words).strip() or spaced.strip() or stem


@dataclass(frozen=True)
class SystemFont:
    """A font file installed on this machine."""

    path: str
    family: str
    style: tuple[bool, bool, bool, bool]


@lru_cache(maxsize=1)
def system_fonts() -> tuple[dict[str, SystemFont], dict[str, dict[tuple, SystemFont]]]:
    """Index the machine's fonts by name and by family.

    Each font is opened once to read the name it calls itself, because file
    names are unreliable: `c0419bt_.pfb` is "Courier 10 Pitch", and `DejaVuSans`
    cannot be split into words by case alone. Both the real name and the file
    name are registered, so a PDF's BaseFont entry finds the font whichever of
    the two it happens to resemble. The whole scan runs once per process.
    """
    by_name: dict[str, SystemFont] = {}
    by_family: dict[str, dict[tuple, SystemFont]] = {}
    seen = 0
    named = 0

    def register(entry: SystemFont, *keys: str) -> None:
        for key in keys:
            for variant in {normalize_font_name(key), canonical_font_name(key)}:
                if variant:
                    by_name.setdefault(variant, entry)
        by_family.setdefault(family_key(entry.family), {}).setdefault(entry.style, entry)

    for directory in _SYSTEM_FONT_DIRS:
        if not os.path.isdir(directory):
            continue
        for root, _dirs, files in os.walk(directory):
            for filename in sorted(files):
                if not filename.lower().endswith(_FONT_EXTENSIONS):
                    continue
                if seen >= _MAX_SYSTEM_FONTS:
                    return by_name, by_family
                seen += 1
                path = os.path.join(root, filename)
                stem = os.path.splitext(filename)[0]

                real_name = None
                if named < _MAX_NAMED_FONTS:
                    named += 1
                    try:
                        real_name = pymupdf.Font(fontfile=path).name or None
                    except Exception:
                        real_name = None

                source_name = real_name or stem
                style = style_of(source_name)
                if real_name and not any(style[2:]):
                    # Some fonts name only the family and leave the cut to the
                    # file name ("Loma.otf" vs "Loma-Bold.otf").
                    style = style_of(f"{source_name} {stem}")

                family = _strip_style_words(real_name) if real_name else _pretty_family(stem)
                if not family:
                    continue
                entry = SystemFont(path=path, family=family, style=style)
                register(entry, source_name, stem)

    return by_name, by_family


def _strip_style_words(name: str) -> str:
    """Drop the trailing weight and slant words from a font's own name.

    "DejaVu Serif Bold Italic" is the family "DejaVu Serif"; the rest is the cut,
    which the toolbar applies with its own bold and italic buttons.
    """
    words = re.split(r"[\s_]+", _SUBSET_PREFIX.sub("", name).strip())
    while len(words) > 1 and normalize_font_name(words[-1]) in _STYLE_WORDS:
        words.pop()
    return " ".join(words).strip()


@lru_cache(maxsize=1)
def _alias_index() -> dict[str, tuple[str, ...]]:
    """The alias table keyed the same way families are keyed.

    ``TimesNewRomanPSMT`` reduces to the family ``timesnew`` — "Roman" is a
    style word in ``Times-Roman`` — so the table's own keys are put through the
    same reduction instead of being matched literally.
    """
    index: dict[str, tuple[str, ...]] = {}
    for name, targets in _METRIC_ALIASES.items():
        reduced = tuple(dict.fromkeys(family_key(target) for target in targets))
        for key in {name, family_key(name)}:
            index.setdefault(key, reduced)
    return index


@dataclass(frozen=True)
class FontMatch:
    """An installed font, and how closely it answers what was asked for."""

    font: SystemFont
    kind: str
    """``name`` the font itself · ``alias`` a metric-compatible stand-in ·
    ``cut`` a different family, chosen to keep the weight or slant asked for."""

    @property
    def is_close_enough(self) -> bool:
        """Whether the result passes for what was asked, without a warning."""
        return self.kind in {"name", "alias"}


def _family_with_cut(
    by_family: dict[str, dict[tuple, SystemFont]], wanted: tuple[bool, bool, bool, bool]
) -> SystemFont | None:
    """Any installed family that has this exact cut, best-known ones first."""
    preferred = ("liberation", "dejavu", "free", "noto")
    candidates = [
        entry
        for variants in by_family.values()
        for entry in variants.values()
        if entry.style == wanted
    ]
    if not candidates:
        return None
    def rank(entry: SystemFont) -> tuple[int, str]:
        key = family_key(entry.family)
        for index, prefix in enumerate(preferred):
            if key.startswith(prefix):
                return (index, entry.family)
        return (len(preferred), entry.family)
    return min(candidates, key=rank)


def find_match(
    name: str | None, style: tuple[bool, bool, bool, bool] | None = None
) -> FontMatch | None:
    """Locate an installed font for the name a PDF refers to it by.

    Tries the name itself, then the metric-compatible stand-ins for fonts
    documents name but do not carry, and finally — rather than silently
    dropping a weight or slant the family has no cut for — another family that
    does have it. Asking for italic and getting upright text is not an answer.
    """
    if not name:
        return None
    by_name, by_family = system_fonts()
    wanted = style or style_of(name)

    for key in (normalize_font_name(name), canonical_font_name(name)):
        found = by_name.get(key)
        if found is not None and found.style[2:] == wanted[2:]:
            return FontMatch(found, "name")

    own = family_key(name)
    aliases = _alias_index().get(own, ())
    wrong_cut: SystemFont | None = None
    for index, key in enumerate((own, *aliases)):
        variants = by_family.get(key)
        if not variants:
            continue
        exact = variants.get(wanted)
        if exact is not None:
            return FontMatch(exact, "name" if index == 0 else "alias")
        if wrong_cut is None:
            wrong_cut = min(
                variants.values(),
                key=lambda entry: sum(a != b for a, b in zip(entry.style, wanted)),
            )

    # No family of the right name has the cut asked for. Keeping the weight or
    # slant matters more than keeping the family, so a family that has it wins
    # over the right family in the wrong cut.
    if wrong_cut is not None and wrong_cut.style[2:] != wanted[2:]:
        replacement = _family_with_cut(by_family, wanted)
        if replacement is not None:
            return FontMatch(replacement, "cut")
    if wrong_cut is not None:
        return FontMatch(wrong_cut, "alias")
    replacement = _family_with_cut(by_family, wanted)
    return FontMatch(replacement, "cut") if replacement is not None else None


def find_system_font(
    name: str | None, style: tuple[bool, bool, bool, bool] | None = None
) -> SystemFont | None:
    """The installed font for ``name``, without saying how it was matched."""
    match = find_match(name, style)
    return match.font if match is not None else None


@lru_cache(maxsize=64)
def _system_font_path(style: tuple[bool, bool, bool, bool]) -> str | None:
    """Locate an installed font file for ``style``, if the machine has one."""
    for candidate in _SYSTEM_FONT_FILES.get(style, ()):  # best match first
        for directory in _SYSTEM_FONT_DIRS:
            path = os.path.join(directory, candidate)
            if os.path.isfile(path):
                return path
    return None


@lru_cache(maxsize=32)
def _load_font_file(path: str) -> tuple[bytes, pymupdf.Font] | None:
    """Read a font file once and keep it; font programs are large."""
    try:
        with open(path, "rb") as handle:
            buffer = handle.read()
        return buffer, pymupdf.Font(fontbuffer=buffer)
    except Exception:
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

        # 2. The font the document *names*, if this machine has it installed.
        #    A PDF may name a font without embedding it, and a PDF that does
        #    embed one may carry a subset missing the characters just typed.
        #    Either way the right answer is that same typeface, not a look-alike
        #    picked by style — which is how "Arial" used to become DejaVu Sans.
        match = find_match(font_name, style)
        if match is not None and match.is_close_enough:
            loaded = _load_font_file(match.font.path)
            if loaded is not None and (not allow_substitution or _covers(loaded[1], text)):
                return ResolvedFont(
                    key=f"sys:{match.font.path}",
                    font=loaded[1],
                    buffer=loaded[0],
                    source="system-named",
                )

        note = (
            f"«{font_name}» no tiene glifos para algunos caracteres nuevos"
            if exact_exists
            else f"no se encontró la fuente «{font_name}» ni en el PDF ni en este equipo"
        )

        # 3. Another program already in the document with the same style.
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
        loaded = _load_font_file(path) if path else None
        if loaded is not None and _covers(loaded[1], text):
            resolved = ResolvedFont(
                key=f"sys:{path}",
                font=loaded[1],
                buffer=loaded[0],
                source="system",
                note=note,
            )
            self._fallback_cache[cache_key] = resolved
            return resolved

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

        # A family the user picked from the toolbar: honour the name, in the
        # weight and slant they asked for.
        match = find_match(family, style)
        if match is not None:
            loaded = _load_font_file(match.font.path)
            if loaded is not None and _covers(loaded[1], text):
                return ResolvedFont(
                    key=f"sys:{match.font.path}",
                    font=loaded[1],
                    buffer=loaded[0],
                    source="system-named" if match.is_close_enough else "system",
                    note=None if match.is_close_enough else (
                        f"«{family}» no tiene "
                        + (" ".join(w for w, on in (("negrita", bold), ("cursiva", italic)) if on)
                           or "ese estilo")
                        + f" en este equipo; se usó «{match.font.family}»"
                    ),
                )
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

        # Everything installed here, so a document whose fonts are not embedded
        # is not left with three generic choices.
        _by_name, by_family = system_fonts()
        installed: dict[str, dict] = {}
        for variants in by_family.values():
            # Describe the family by its plain cut when it has one.
            entry = min(variants.values(), key=lambda item: (item.style[2], item.style[3]))
            if entry.family and entry.family not in installed and entry.family not in seen:
                serif, mono, bold, italic = entry.style
                installed[entry.family] = {
                    "name": entry.family,
                    "source": "system",
                    "bold": False,
                    "italic": False,
                    "mono": mono,
                    "serif": serif,
                }
        families.extend(sorted(installed.values(), key=lambda item: item["name"].lower()))

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
