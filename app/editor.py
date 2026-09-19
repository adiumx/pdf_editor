"""Applying edits back onto the page.

Every text edit is the same two steps: erase the original glyphs with a
redaction, then draw the replacement at the baseline the original sat on, with
the font program it was drawn with. Doing it in that order — and doing all of a
page's erasures before any of its redrawing — is what keeps an edit from eating
the text an earlier edit just wrote.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Iterable, Sequence

import pymupdf

from .extract import hex_to_pdf, page_rotation_of, to_page
from .fonts import FLAG_BOLD, FLAG_ITALIC, FontResolver, ResolvedFont

# Redaction rectangles are grown by this much so no anti-aliased sliver of the
# old glyphs survives, and no more, so neighbouring lines are left alone.
_REDACT_PAD = 0.4

# A replacement is never shrunk past this fraction of its original size; below
# it the result looks wrong enough that overflowing is the better answer.
_MIN_SHRINK = 0.5


class EditError(ValueError):
    """An operation that cannot be carried out as described."""


@dataclass
class EditWarning:
    """Something the user should know about an edit that still went through."""

    page: int
    kind: str
    message: str

    def as_dict(self) -> dict[str, Any]:
        return {"page": self.page, "kind": self.kind, "message": self.message}


@dataclass
class _PendingText:
    """One run of text waiting to be drawn once the page has been erased."""

    page: int
    point: tuple[float, float]
    text: str
    resolved: ResolvedFont
    fontsize: float
    color: tuple[float, float, float]
    rotate: int
    opacity: float = 1.0


@dataclass
class _PageBatch:
    """Everything one page needs doing, in the only order that is safe."""

    redactions: list[pymupdf.Rect] = field(default_factory=list)
    texts: list[_PendingText] = field(default_factory=list)
    images: list[tuple[pymupdf.Rect, bytes, int]] = field(default_factory=list)


def _axis_extent(bbox: Sequence[float], rotation: int) -> float:
    """Length of a line's box along the direction the text runs."""
    if rotation in (90, 270):
        return float(bbox[3]) - float(bbox[1])
    return float(bbox[2]) - float(bbox[0])


def _start_offset(align: str, extent: float, needed: float) -> float:
    """How far into the original box the re-flowed text should start."""
    slack = extent - needed
    if align == "center":
        return slack / 2
    if align == "right":
        return slack
    return 0.0


def _advance(point: tuple[float, float], distance: float, rotation: int) -> tuple[float, float]:
    """Move ``distance`` points along the writing direction."""
    x, y = point
    if rotation == 90:
        return (x, y - distance)
    if rotation == 180:
        return (x - distance, y)
    if rotation == 270:
        return (x, y + distance)
    return (x + distance, y)


def _line_start(
    bbox: Sequence[float], origin: Sequence[float], rotation: int, offset: float
) -> tuple[float, float]:
    """The baseline point the first span should be drawn at."""
    x0, y0, x1, y1 = (float(v) for v in bbox)
    ox, oy = float(origin[0]), float(origin[1])
    if rotation == 90:
        return (ox, y1 - offset)
    if rotation == 180:
        return (x1 - offset, oy)
    if rotation == 270:
        return (ox, y0 + offset)
    return (x0 + offset, oy)


def _span_font(
    resolver: FontResolver, pno: int, span: dict[str, Any]
) -> ResolvedFont:
    """Resolve the font for one span of an edit request.

    A span may ask for a specific family (the toolbar changed it) or simply
    carry the name it was extracted with (the text alone was edited).
    """
    text = span.get("text", "")
    family = span.get("family")
    if family:
        return resolver.resolve_family(
            pno,
            family,
            bold=bool(span.get("bold")),
            italic=bool(span.get("italic")),
            text=text,
        )
    flags = int(span.get("flags", 0))
    if span.get("bold"):
        flags |= FLAG_BOLD
    if span.get("italic"):
        flags |= FLAG_ITALIC
    return resolver.resolve(pno, span.get("font"), flags, text)


class EditSession:
    """Collects operations for one request and applies them to the document."""

    def __init__(self, doc: pymupdf.Document, resolver: FontResolver) -> None:
        self._doc = doc
        self._resolver = resolver
        self._batches: dict[int, _PageBatch] = {}
        self.warnings: list[EditWarning] = []

    # -- collection --------------------------------------------------------

    def _batch(self, pno: int) -> _PageBatch:
        if pno < 0 or pno >= self._doc.page_count:
            raise EditError(f"La página {pno + 1} no existe")
        return self._batches.setdefault(pno, _PageBatch())

    def _warn(self, pno: int, kind: str, message: str) -> None:
        self.warnings.append(EditWarning(pno, kind, message))

    # The client works in the coordinate space of the rendered image, which
    # honours the page's /Rotate entry; drawing happens in the page's own space,
    # which does not. On an unrotated page these are the same and cost nothing.

    def _rect(self, pno: int, values: Sequence[float]) -> pymupdf.Rect:
        page = self._doc[pno]
        rect = pymupdf.Rect(values)
        if page.rotation:
            rect = to_page(page, rect)
            rect.normalize()
        return rect

    def _point(self, pno: int, values: Sequence[float]) -> tuple[float, float]:
        page = self._doc[pno]
        point = pymupdf.Point(values[0], values[1])
        if page.rotation:
            point = to_page(page, point)
        return (point.x, point.y)

    def _rotation(self, pno: int, rotation: int) -> int:
        return page_rotation_of(self._doc[pno].rotation, int(rotation) % 360)

    def replace_line(self, op: dict[str, Any]) -> None:
        """Rewrite one line, re-flowing its spans from the original baseline."""
        pno = int(op["page"])
        batch = self._batch(pno)
        bbox = list(self._rect(pno, op["bbox"]))
        raw_origin = op.get("origin") or [op["bbox"][0], op["bbox"][3]]
        origin = self._point(pno, raw_origin)
        rotation = self._rotation(pno, op.get("rotation", 0))
        align = op.get("align", "left")
        fit = op.get("fit", "overflow")
        spans = [s for s in op.get("spans", []) if s.get("text")]

        batch.redactions.append(pymupdf.Rect(bbox) + (-_REDACT_PAD, -_REDACT_PAD, _REDACT_PAD, _REDACT_PAD))
        if not spans:
            return  # deleting the line is just the redaction

        resolved = [(_span_font(self._resolver, pno, span), span) for span in spans]
        for font, span in resolved:
            if font.note:
                self._warn(pno, "font-substituted", font.note)
            missing = font.missing_glyphs(span["text"])
            if missing:
                self._warn(
                    pno,
                    "missing-glyphs",
                    "No se pudieron dibujar estos caracteres: " + " ".join(missing),
                )

        sizes = [float(span.get("size", 11.0)) for _font, span in resolved]
        needed = sum(font.text_length(span["text"], size) for (font, span), size in zip(resolved, sizes))
        extent = _axis_extent(bbox, rotation)

        if fit == "shrink" and needed > extent > 0:
            ratio = max(extent / needed, _MIN_SHRINK)
            sizes = [size * ratio for size in sizes]
            needed = sum(
                font.text_length(span["text"], size) for (font, span), size in zip(resolved, sizes)
            )
            if ratio == _MIN_SHRINK:
                self._warn(
                    pno,
                    "overflow",
                    "El texto nuevo es mucho más largo que el original y sobresale del renglón.",
                )
        elif needed > extent + 1 and extent > 0:
            self._warn(
                pno,
                "overflow",
                "El texto nuevo es más ancho que el original; usa «ajustar» si no quieres que sobresalga.",
            )

        point = _line_start(bbox, origin, rotation, _start_offset(align, extent, needed))
        for (font, span), size in zip(resolved, sizes):
            batch.texts.append(
                _PendingText(
                    page=pno,
                    point=point,
                    text=span["text"],
                    resolved=font,
                    fontsize=size,
                    color=hex_to_pdf(span.get("color", "#000000")),
                    rotate=rotation,
                    opacity=float(span.get("alpha", 255)) / 255.0,
                )
            )
            point = _advance(point, font.text_length(span["text"], size), rotation)

    def delete_line(self, op: dict[str, Any]) -> None:
        """Erase a line without putting anything back."""
        self.replace_line({**op, "spans": []})

    def add_text(self, op: dict[str, Any]) -> None:
        """Draw a new block of text inside a rectangle the user drew."""
        pno = int(op["page"])
        batch = self._batch(pno)
        rect = self._rect(pno, op["rect"])
        text = str(op.get("text", ""))
        if not text.strip():
            return
        size = float(op.get("size", 12.0))
        rotation = self._rotation(pno, op.get("rotation", 0))
        align = op.get("align", "left")
        span_spec = {
            "text": text,
            "family": op.get("family", "sans"),
            "font": op.get("font"),
            "bold": op.get("bold", False),
            "italic": op.get("italic", False),
            "size": size,
        }
        font = _span_font(self._resolver, pno, span_spec)
        if font.note:
            self._warn(pno, "font-substituted", font.note)
        missing = font.missing_glyphs(text)
        if missing:
            self._warn(pno, "missing-glyphs", "Caracteres no disponibles: " + " ".join(missing))

        color = hex_to_pdf(op.get("color", "#000000"))
        line_height = size * float(op.get("line_height", 1.25))
        ascent = font.font.ascender * size
        width = rect.width if rotation in (0, 180) else rect.height

        for index, chunk in enumerate(text.split("\n")):
            if not chunk:
                continue
            needed = font.text_length(chunk, size)
            offset = _start_offset(align, width, needed)
            if rotation == 90:
                point = (rect.x0 + ascent + index * line_height, rect.y1 - offset)
            elif rotation == 180:
                point = (rect.x1 - offset, rect.y1 - ascent - index * line_height)
            elif rotation == 270:
                point = (rect.x1 - ascent - index * line_height, rect.y0 + offset)
            else:
                point = (rect.x0 + offset, rect.y0 + ascent + index * line_height)
            batch.texts.append(
                _PendingText(
                    page=pno,
                    point=point,
                    text=chunk,
                    resolved=font,
                    fontsize=size,
                    color=color,
                    rotate=rotation,
                )
            )

    def erase_area(self, op: dict[str, Any]) -> None:
        """Wipe everything inside a rectangle the user drew."""
        pno = int(op["page"])
        self._batch(pno).redactions.append(self._rect(pno, op["rect"]))

    def insert_image(self, op: dict[str, Any], data: bytes) -> None:
        """Place an uploaded image inside a rectangle."""
        pno = int(op["page"])
        batch = self._batch(pno)
        batch.images.append(
            (self._rect(pno, op["rect"]), data, self._rotation(pno, op.get("rotate", 0)))
        )

    # -- application -------------------------------------------------------

    def commit(self) -> list[EditWarning]:
        """Erase, then redraw, page by page."""
        for pno, batch in self._batches.items():
            page = self._doc[pno]
            if batch.redactions:
                for rect in batch.redactions:
                    if not rect.is_empty:
                        page.add_redact_annot(rect)
                page.apply_redactions(
                    images=pymupdf.PDF_REDACT_IMAGE_NONE,
                    graphics=pymupdf.PDF_REDACT_LINE_ART_NONE,
                    text=pymupdf.PDF_REDACT_TEXT_REMOVE,
                )
                # The content stream was rewritten; page font resources with it.
                self._resolver.forget_page(pno)

            for pending in batch.texts:
                fontname = self._resolver.install(page, pending.resolved)
                try:
                    page.insert_text(
                        pending.point,
                        pending.text,
                        fontname=fontname,
                        fontsize=pending.fontsize,
                        color=pending.color,
                        rotate=pending.rotate,
                        fill_opacity=pending.opacity,
                        overlay=True,
                    )
                except Exception as exc:  # pragma: no cover - defensive
                    raise EditError(f"No se pudo escribir el texto: {exc}") from exc

            for rect, data, rotate in batch.images:
                try:
                    page.insert_image(rect, stream=data, rotate=rotate, keep_proportion=True)
                except Exception as exc:
                    raise EditError(f"No se pudo insertar la imagen: {exc}") from exc

        return self.warnings


def apply_operations(
    doc: pymupdf.Document,
    resolver: FontResolver,
    operations: Iterable[dict[str, Any]],
    assets: dict[str, bytes] | None = None,
) -> list[EditWarning]:
    """Apply a batch of operations to ``doc`` in place.

    Text operations for a page are batched so all of its erasures happen before
    any of its redrawing. Page-structure operations run last, once no operation
    still refers to a page by its old index.
    """
    assets = assets or {}
    session = EditSession(doc, resolver)
    structural: list[dict[str, Any]] = []

    for op in operations:
        kind = op.get("op")
        if kind == "replace_line":
            session.replace_line(op)
        elif kind == "delete_line":
            session.delete_line(op)
        elif kind == "add_text":
            session.add_text(op)
        elif kind == "erase_area":
            session.erase_area(op)
        elif kind == "insert_image":
            asset_id = op.get("asset")
            if asset_id not in assets:
                raise EditError("La imagen ya no está disponible; vuelve a subirla.")
            session.insert_image(op, assets[asset_id])
        elif kind in {"delete_page", "move_page", "rotate_page", "insert_page"}:
            structural.append(op)
        else:
            raise EditError(f"Operación desconocida: {kind!r}")

    warnings = session.commit()
    for op in structural:
        _apply_structural(doc, op)
    return warnings


def _apply_structural(doc: pymupdf.Document, op: dict[str, Any]) -> None:
    """Delete, move, rotate or add a page."""
    kind = op["op"]
    if kind == "delete_page":
        pno = int(op["page"])
        if doc.page_count <= 1:
            raise EditError("No se puede borrar la única página del documento.")
        if not 0 <= pno < doc.page_count:
            raise EditError(f"La página {pno + 1} no existe")
        doc.delete_page(pno)
    elif kind == "move_page":
        pno = int(op["page"])
        count = doc.page_count
        if not 0 <= pno < count:
            raise EditError(f"La página {pno + 1} no existe")
        # `to` is the index to insert *before*; PyMuPDF spells "after the last
        # page" as -1 rather than as `count`, which is what a caller dragging a
        # page to the bottom of the rail naturally produces.
        to = int(op["to"])
        if not 0 <= to < count:
            to = -1
        if to == -1:
            if pno == count - 1:
                return
        elif to in (pno, pno + 1):
            return
        doc.move_page(pno, to)
    elif kind == "rotate_page":
        pno = int(op["page"])
        if not 0 <= pno < doc.page_count:
            raise EditError(f"La página {pno + 1} no existe")
        doc[pno].set_rotation(int(op.get("rotation", 0)) % 360)
    elif kind == "insert_page":
        at = int(op.get("at", doc.page_count))
        reference = doc[min(max(at - 1, 0), doc.page_count - 1)].rect
        doc.new_page(pno=at, width=reference.width, height=reference.height)
