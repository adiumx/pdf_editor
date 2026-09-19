"""Recognising the text in a scanned page.

A scan has no text: it has a picture of text. Nothing in the editor can touch
it, because there is nothing there to touch. Recognition puts a text layer over
the picture, and from then on the page edits like any other — with one
difference that matters: erasing a line has to blank the pixels underneath as
well, or the scanned word stays visible under whatever replaces it.
"""

from __future__ import annotations

import functools
import shutil
import subprocess
from dataclasses import dataclass

import pymupdf

# Marker written into the page's own dictionary, so it survives reordering,
# saving and reopening. A scanned page needs its pixels blanked when edited;
# a page that merely has a large picture on it does not.
OCR_MARK = "PDFEditorOCR"

# Recognition runs on a render of the page, not on the page. More dots mean a
# better reading and a slower one; 300 is the usual compromise.
DEFAULT_DPI = 300

# Below this many characters a page is treated as having no text of its own.
_TEXT_THRESHOLD = 24

# How much of the page a picture has to cover for the page to look like a scan.
_SCAN_COVERAGE = 0.6


class OcrUnavailable(RuntimeError):
    """Tesseract is not installed, so nothing can be recognised."""


@dataclass(frozen=True)
class OcrSupport:
    """What this machine can recognise."""

    available: bool
    languages: tuple[str, ...] = ()
    detail: str = ""

    @property
    def default_language(self) -> str:
        for preferred in ("spa", "eng"):
            if preferred in self.languages:
                return preferred
        return self.languages[0] if self.languages else "eng"

    def as_dict(self) -> dict:
        return {
            "available": self.available,
            "languages": list(self.languages),
            "default": self.default_language if self.available else None,
            "detail": self.detail,
        }


@functools.lru_cache(maxsize=1)
def support() -> OcrSupport:
    """Whether Tesseract is installed, and which languages it has."""
    binary = shutil.which("tesseract")
    if not binary:
        return OcrSupport(
            available=False,
            detail=(
                "Tesseract no está instalado. En Debian o Ubuntu: "
                "sudo apt install tesseract-ocr tesseract-ocr-spa"
            ),
        )
    try:
        result = subprocess.run(
            [binary, "--list-langs"], capture_output=True, text=True, timeout=20, check=False
        )
        languages = tuple(
            line.strip()
            for line in result.stdout.splitlines()[1:]
            if line.strip() and " " not in line.strip()
        )
    except Exception as exc:  # pragma: no cover - defensive
        return OcrSupport(available=False, detail=f"No se pudo consultar Tesseract: {exc}")

    if not languages:
        return OcrSupport(
            available=False,
            detail="Tesseract está instalado pero sin ningún idioma. Instala, por "
            "ejemplo, tesseract-ocr-spa.",
        )
    return OcrSupport(available=True, languages=languages)


def is_scanned(page: pymupdf.Page) -> bool:
    """Whether the page is a picture of a document rather than a document."""
    try:
        infos = page.get_image_info()
    except Exception:
        return False
    area = abs(page.rect.get_area())
    if area <= 0:
        return False
    for info in infos:
        if abs(pymupdf.Rect(info["bbox"]).get_area()) / area >= _SCAN_COVERAGE:
            return True
    return False


def has_text(page: pymupdf.Page) -> bool:
    """Whether the page has enough text to be worth editing as it stands."""
    try:
        return len(page.get_text().strip()) >= _TEXT_THRESHOLD
    except Exception:
        return False


def needs_ocr(page: pymupdf.Page) -> bool:
    """A page that is a scan and has no text layer yet."""
    return is_scanned(page) and not has_text(page)


def was_recognised(page: pymupdf.Page) -> bool:
    """Whether this page's text came from recognition.

    Read from the page's own dictionary rather than remembered on the side, so
    it still holds after pages are reordered, or the file is saved and reopened.
    """
    try:
        kind, value = page.parent.xref_get_key(page.xref, OCR_MARK)
    except Exception:
        return False
    return kind != "null" and str(value).lower() in {"true", "/true", "(true)"}


def _mark(page: pymupdf.Page) -> None:
    try:
        page.parent.xref_set_key(page.xref, OCR_MARK, "true")
    except Exception:  # pragma: no cover - the mark is a convenience
        pass


def recognise_page(
    doc: pymupdf.Document, pno: int, *, language: str | None = None, dpi: int = DEFAULT_DPI
) -> bool:
    """Put a text layer on one page. Returns whether anything was recognised."""
    capability = support()
    if not capability.available:
        raise OcrUnavailable(capability.detail)

    page = doc[pno]
    rect, rotation = pymupdf.Rect(page.rect), page.rotation
    try:
        pixmap = page.get_pixmap(dpi=dpi)
        recognised = pixmap.pdfocr_tobytes(language=language or capability.default_language)
    except Exception as exc:
        raise OcrUnavailable(f"El reconocimiento falló: {exc}") from exc

    with pymupdf.open(stream=recognised, filetype="pdf") as layer:
        if not layer[0].get_text().strip():
            return False
        doc.insert_pdf(layer, start_at=pno)

    # insert_pdf put the recognised page just before the original; the original
    # goes now, and the replacement takes its size and rotation.
    doc.delete_page(pno + 1)
    fresh = doc[pno]
    fresh.set_mediabox(rect)
    if rotation:
        fresh.set_rotation(rotation)
    _mark(fresh)
    return True


def recognise(
    doc: pymupdf.Document,
    pages: list[int] | None = None,
    *,
    language: str | None = None,
    dpi: int = DEFAULT_DPI,
) -> list[int]:
    """Recognise the given pages, or every page that needs it. Returns which."""
    targets = pages if pages is not None else [
        pno for pno in range(doc.page_count) if needs_ocr(doc[pno])
    ]
    done = []
    for pno in targets:
        if not 0 <= pno < doc.page_count:
            continue
        if recognise_page(doc, pno, language=language, dpi=dpi):
            done.append(pno)
    return done
