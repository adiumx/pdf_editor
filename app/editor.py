"""Applying edits back onto the page.

Every text edit is the same two steps: erase the original glyphs with a
redaction, then draw the replacement at the baseline the original sat on, with
the font program it was drawn with. Doing it in that order — and doing all of a
page's erasures before any of its redrawing — is what keeps an edit from eating
the text an earlier edit just wrote.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any, Iterable, Sequence

import pymupdf

from .extract import (
    across,
    across_span,
    along,
    along_span,
    from_axes,
    hex_to_pdf,
    page_rotation_of,
    to_page,
)
from .fonts import FLAG_BOLD, FLAG_ITALIC, FontResolver, ResolvedFont
from .ocr import was_recognised

# Redaction rectangles are grown by this much so no anti-aliased sliver of the
# old glyphs survives, and no more, so neighbouring lines are left alone.
_REDACT_PAD = 0.4

# A replacement is never shrunk past this fraction of its original size; below
# it the result looks wrong enough that overflowing is the better answer.
_MIN_SHRINK = 0.5

# How far a paragraph's line spacing may be squeezed to keep it in the room it
# has. Tightening the leading a little is far less visible than shrinking the
# text, so it is tried first; past this the letters have to give instead.
_MIN_LEADING = 0.86


@dataclass
class _Token:
    """A word or a run of spaces, with where it was and how it is set."""

    text: str
    run: int
    is_space: bool
    width: float
    source_line: int = -1
    source_from: float = 0.0
    source_to: float = 0.0
    placed_line: int = -1
    placed_from: float = 0.0
    placed_to: float = 0.0


@dataclass
class _Run:
    """A stretch of text set one way, as it goes into the paragraph."""

    text: str
    font: ResolvedFont
    size: float
    color: tuple[float, float, float]
    opacity: float = 1.0


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


def _scale_matrix(hscale: float, rotation: int) -> pymupdf.Matrix:
    """Squeeze text along the direction it runs, leaving its height alone."""
    if rotation in (90, 270):
        return pymupdf.Matrix(1, 0, 0, hscale, 0, 0)
    return pymupdf.Matrix(hscale, 0, 0, 1, 0, 0)


@dataclass
class _LineRewrite:
    """Where a line was, and where its replacement ended up.

    Redaction takes the page's links with it, so anything anchored to a line —
    a hyperlink over an email address or a repository URL — has to be put back
    over the text that replaced it.
    """

    old: pymupdf.Rect
    new: pymupdf.Rect
    rotation: int


@dataclass
class _PageBatch:
    """Everything one page needs doing, in the only order that is safe."""

    redactions: list[pymupdf.Rect] = field(default_factory=list)
    texts: list[_PendingText] = field(default_factory=list)
    images: list[tuple[pymupdf.Rect, bytes, int]] = field(default_factory=list)
    rewrites: list[_LineRewrite] = field(default_factory=list)
    new_links: list[dict[str, Any]] = field(default_factory=list)
    drawings: list[tuple[dict[str, Any], pymupdf.Point]] = field(default_factory=list)
    remove_line_art: bool = False
    """Set when something has to move: line art is then erased along with the
    text and put back where it belongs, instead of being left behind."""

    restored_images: list[tuple[pymupdf.Rect, bytes]] = field(default_factory=list)
    moved_image_sources: set[tuple] = field(default_factory=set)
    carried_links: set[tuple] = field(default_factory=set)
    """Links already taken to a continuation page, so the ordinary shift does
    not add them a second time."""
    move_images: bool = False
    """Set when a picture has to move. Erasing images is all or nothing over a
    page's redactions, so every one they touch is put back — shifted if it is
    in the way, where it was if it is not."""


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


def _drawn_rect(
    start: tuple[float, float], length: float, old: Sequence[float], rotation: int
) -> pymupdf.Rect:
    """The box the re-flowed text occupies, given where it starts and its length."""
    x, y = start
    x0, y0, x1, y1 = (float(v) for v in old)
    if rotation == 90:
        return pymupdf.Rect(x0, y - length, x1, y)
    if rotation == 180:
        return pymupdf.Rect(x - length, y0, x, y1)
    if rotation == 270:
        return pymupdf.Rect(x0, y, x1, y + length)
    return pymupdf.Rect(x, y0, x + length, y1)


def _remap(
    rect: pymupdf.Rect, rewrite: _LineRewrite
) -> pymupdf.Rect:
    """Move a rectangle anchored to a line onto that line's replacement.

    Its position along the writing direction is kept as a fraction of the line,
    so a link over the third word stays over the third word when the line grows
    or shrinks.
    """
    old, new = rewrite.old, rewrite.new
    vertical = rewrite.rotation in (90, 270)
    span = (old.y1 - old.y0) if vertical else (old.x1 - old.x0)
    if span <= 0:
        return pymupdf.Rect(new)
    if vertical:
        start = (rect.y0 - old.y0) / span
        end = (rect.y1 - old.y0) / span
        height = new.y1 - new.y0
        return pymupdf.Rect(new.x0, new.y0 + start * height, new.x1, new.y0 + end * height)
    start = (rect.x0 - old.x0) / span
    end = (rect.x1 - old.x0) / span
    width = new.x1 - new.x0
    return pymupdf.Rect(new.x0 + start * width, new.y0, new.x0 + end * width, new.y1)


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


def _tokenize(runs: list[_Run]) -> list[_Token]:
    """Break the paragraph into the pieces a line can be broken between."""
    tokens: list[_Token] = []
    for index, run in enumerate(runs):
        for piece in re.split(r"(\s+)", run.text):
            if not piece:
                continue
            tokens.append(
                _Token(
                    text=piece,
                    run=index,
                    is_space=piece.isspace(),
                    width=run.font.text_length(piece, run.size),
                )
            )
    return tokens


# A measured width and a reported box never agree exactly: a span's box ends at
# its last glyph, while measuring adds that glyph's full advance. On a full line
# the difference — a third of a point — is enough to push its last word onto a
# line of its own, so re-breaking a paragraph nobody edited would change where
# its lines end. A fraction of a point of slack is invisible and prevents it.
_WRAP_SLACK = 0.002
_WRAP_SLACK_MIN = 0.5


def _wrap(tokens: list[_Token], first_width: float, width: float) -> list[list[_Token]]:
    """Greedily break tokens into lines, the way a text engine does.

    The first line may be narrower than the rest, because a paragraph's opening
    line is often indented.
    """
    def slack(limit: float) -> float:
        return max(_WRAP_SLACK_MIN, limit * _WRAP_SLACK)

    lines: list[list[_Token]] = []
    current: list[_Token] = []
    used = 0.0
    limit = max(first_width, 1.0)

    for token in tokens:
        if token.is_space and not current:
            continue  # a wrapped line does not start with a space
        if not token.is_space and current and used + token.width > limit + slack(limit):
            while current and current[-1].is_space:
                used -= current.pop().width
            lines.append(current)
            current, used, limit = [], 0.0, max(width, 1.0)
        current.append(token)
        used += token.width

    while current and current[-1].is_space:
        current.pop()
    if current:
        lines.append(current)
    return lines


def _group_into_runs(line: list[_Token]) -> list[tuple[int, str]]:
    """Merge a wrapped line's tokens back into one piece per style."""
    grouped: list[tuple[int, str]] = []
    for token in line:
        if grouped and grouped[-1][0] == token.run:
            grouped[-1] = (token.run, grouped[-1][1] + token.text)
        else:
            grouped.append((token.run, token.text))
    return grouped


def _visible_opacity(alpha: Any) -> float:
    """The opacity to draw a replacement at.

    Fully transparent is never the answer to an edit: the text layer over a
    scan is invisible on purpose, because the words the reader sees are pixels
    underneath it. Writing the replacement invisibly too would erase the line
    and put nothing in its place.
    """
    try:
        value = float(alpha) / 255.0
    except (TypeError, ValueError):
        return 1.0
    return 1.0 if value <= 0.01 else min(value, 1.0)


def _redraw(page: pymupdf.Page, drawing: dict[str, Any], shift: pymupdf.Point) -> None:
    """Put a simple drawing back on the page, moved by ``shift``."""
    colour = drawing.get("color")
    fill = drawing.get("fill")
    width = drawing.get("width") or 1.0
    for item in drawing.get("items", []):
        try:
            if item[0] == "l":
                page.draw_line(item[1] + shift, item[2] + shift, color=colour, width=width)
            elif item[0] == "re":
                rect = pymupdf.Rect(item[1]) + (shift.x, shift.y, shift.x, shift.y)
                page.draw_rect(rect, color=colour, fill=fill, width=width)
            elif item[0] == "c":
                page.draw_bezier(
                    item[1] + shift, item[2] + shift, item[3] + shift, item[4] + shift,
                    color=colour, fill=fill, width=width,
                )
            elif item[0] == "qu":
                quad = pymupdf.Quad(
                    item[1].ul + shift, item[1].ur + shift,
                    item[1].ll + shift, item[1].lr + shift,
                )
                page.draw_quad(quad, color=colour, fill=fill, width=width)
        except Exception:
            continue


def _median_leading(
    origins: list[tuple[float, float]], boxes: list[pymupdf.Rect], rotation: int = 0
) -> float:
    """The paragraph's own line spacing, read from its baselines."""
    positions = [across(rotation, *origin) for origin in origins]
    deltas = sorted(b - a for a, b in zip(positions, positions[1:]) if b - a > 0)
    if deltas:
        return deltas[len(deltas) // 2]
    heights = [across_span(rotation, box) for box in boxes]
    return max((high - low for low, high in heights), default=12.0) * 1.2


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
        """Rewrite one line, re-flowing its spans from the original baseline.

        When only the tail of a line changed and the line starts at a fixed
        edge, the spans before the change are left exactly as they are: redrawing
        them would re-render text nobody edited, which on a document whose fonts
        cannot be reused means watching untouched words change typeface.
        """
        pno = int(op["page"])
        batch = self._batch(pno)
        bbox = list(self._rect(pno, op["bbox"]))
        raw_origin = op.get("origin") or [op["bbox"][0], op["bbox"][3]]
        origin = self._point(pno, raw_origin)
        rotation = self._rotation(pno, op.get("rotation", 0))
        align = op.get("align", "left")
        fit = op.get("fit", "overflow")
        spans = [s for s in op.get("spans", []) if s.get("text")]

        keep = self._untouched_prefix(op, spans, align)
        if keep:
            spans = spans[keep:]
            bbox, origin = self._segment(pno, op["spans"][keep], bbox, rotation)

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
        batch.rewrites.append(
            _LineRewrite(
                old=pymupdf.Rect(bbox),
                new=_drawn_rect(point, needed, bbox, rotation),
                rotation=rotation,
            )
        )
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
                    opacity=_visible_opacity(span.get("alpha", 255)),
                )
            )
            point = _advance(point, font.text_length(span["text"], size), rotation)

    @staticmethod
    def _untouched_prefix(op: dict[str, Any], spans: list[dict], align: str) -> int:
        """How many leading spans can be left alone.

        Only with a left-anchored line: centring or right-aligning moves every
        span when the total width changes, so none of them can stay put.
        """
        first = int(op.get("from_span", 0) or 0)
        if first <= 0 or align != "left":
            return 0
        if first >= len(spans) or first >= len(op.get("spans", [])):
            return 0
        anchor = op["spans"][first]
        if not anchor.get("bbox") or not anchor.get("origin"):
            return 0
        return first

    def _segment(
        self, pno: int, anchor: dict[str, Any], bbox: list[float], rotation: int
    ) -> tuple[list[float], tuple[float, float]]:
        """The part of a line that starts at ``anchor`` and runs to its end."""
        box = list(self._rect(pno, anchor["bbox"]))
        origin = self._point(pno, anchor["origin"])
        x0, y0, x1, y1 = bbox
        if rotation == 90:
            return [x0, y0, x1, box[3]], origin
        if rotation == 180:
            return [x0, y0, box[2], y1], origin
        if rotation == 270:
            return [x0, box[1], x1, y1], origin
        return [box[0], y0, x1, y1], origin

    def replace_paragraph(self, op: dict[str, Any]) -> None:
        """Rewrite a paragraph, re-wrapping it across its lines.

        Editing one line of running text used to leave it overflowing into the
        margin, because nothing pulled the extra words down into the lines
        below. Here the paragraph is rebuilt as one stream of styled words and
        broken again at its own measure.

        The cheap path still applies: while the edited line fits between its
        start and the paragraph's right margin, only that line is touched, which
        leaves the rest of the paragraph on the page untouched.
        """
        pno = int(op["page"])
        lines = op.get("lines") or []
        edited = op.get("edited") or {}
        if not lines:
            raise EditError("El párrafo no tiene líneas")

        index = max(0, min(int(edited.get("line", 0)), len(lines) - 1))
        align = op.get("align", "left")
        box = self._rect(pno, op.get("box") or lines[0]["bbox"])

        single = len(lines) == 1
        forced = bool(op.get("reflow"))
        measure_point = op.get("measure_point")

        if not forced and (
            single or self._line_still_fits(pno, lines[index], box, align, measure_point)
        ):
            # Nothing has to move but the line being edited.
            self.replace_line({**lines[index], "op": "replace_line", "page": pno,
                               "align": align, "fit": op.get("fit", "overflow"),
                               "from_span": edited.get("span", 0)})
            return

        self._reflow(pno, lines, box, align, op.get("fit", "overflow"), measure_point)

    def _line_still_fits(
        self,
        pno: int,
        line: dict[str, Any],
        box: pymupdf.Rect,
        align: str,
        measure_point: Sequence[float] | None = None,
    ) -> bool:
        """Whether the edited line stays inside the paragraph's far margin."""
        if align != "left":
            return False
        spans = [s for s in line.get("spans", []) if s.get("text")]
        if not spans:
            return True
        needed = sum(
            _span_font(self._resolver, pno, span).text_length(
                span["text"], float(span.get("size", 11.0))
            )
            for span in spans
        )
        rotation = self._rotation(pno, line.get("rotation", 0))
        start = along_span(rotation, self._rect(pno, line["bbox"]))[0]
        limit = (
            along(rotation, *self._point(pno, measure_point))
            if measure_point
            else along_span(rotation, box)[1]
        )
        return needed <= (limit - start) + 0.5

    def _paragraph_stream(
        self, pno: int, lines: list[dict[str, Any]], rotation: int = 0
    ) -> tuple[list[_Run], list[_Token]]:
        """Flatten a paragraph into styled runs and the words that make it up.

        Each word keeps where it was on the page, so anything anchored to it —
        a hyperlink — can be found again after the paragraph is re-broken.
        """
        runs: list[_Run] = []
        tokens: list[_Token] = []

        for position, line in enumerate(lines):
            cursor = along_span(rotation, self._rect(pno, line["bbox"]))[0]
            for span in line.get("spans", []):
                text = span.get("text", "")
                if not text:
                    continue
                font = _span_font(self._resolver, pno, span)
                if font.note:
                    self._warn(pno, "font-substituted", font.note)
                size = float(span.get("size", 11.0))
                runs.append(
                    _Run(
                        text=text,
                        font=font,
                        size=size,
                        color=hex_to_pdf(span.get("color", "#000000")),
                        opacity=_visible_opacity(span.get("alpha", 255)),
                    )
                )
                run_index = len(runs) - 1
                for piece in re.split(r"(\s+)", text):
                    if not piece:
                        continue
                    width = font.text_length(piece, size)
                    tokens.append(
                        _Token(
                            text=piece, run=run_index, is_space=piece.isspace(),
                            width=width, source_line=position,
                            source_from=cursor, source_to=cursor + width,
                        )
                    )
                    cursor += width

            # Lines of one paragraph are joined by a space, which is what the
            # break between them stood for — unless one is already there, or the
            # line broke mid-word on a hyphen.
            last = tokens[-1] if tokens else None
            if position < len(lines) - 1 and last is not None and not last.is_space:
                if not last.text.endswith("-"):
                    run = runs[last.run]
                    tokens.append(
                        _Token(text=" ", run=last.run, is_space=True,
                               width=run.font.text_length(" ", run.size),
                               source_line=position, source_from=cursor, source_to=cursor)
                    )
        return runs, tokens

    def _room_below(
        self, pno: int, boxes: list[pymupdf.Rect], rotation: int
    ) -> float:
        """How far the paragraph may reach before it runs into something.

        Measured across the lines, in the paragraph's own direction, so it works
        the same for text that reads upwards. Nothing is moved aside on its own,
        so growing into the next paragraph is a real collision; what is actually
        below is worth looking up, because a paragraph with white space under it
        can simply take it.
        """
        page = self._doc[pno]
        page_limit = max(
            across(rotation, x, y)
            for x, y in ((page.rect.x0, page.rect.y0), (page.rect.x1, page.rect.y1))
        )
        floor = page_limit - 18  # a hair inside the edge of the sheet
        spans = [across_span(rotation, box) for box in boxes]
        bottom = max(span[1] for span in spans)
        reach = [along_span(rotation, box) for box in boxes]
        near, far = min(r[0] for r in reach), max(r[1] for r in reach)

        try:
            raw = page.get_text("dict")
        except Exception:
            return floor

        obstacles = [
            line.get("bbox", (0, 0, 0, 0))
            for block in raw.get("blocks", [])
            if block.get("type") == 0
            for line in block.get("lines", [])
        ]
        # A figure or a picture is as much in the way as a line of text is.
        try:
            obstacles += [drawing["rect"] for drawing in page.get_drawings()]
            obstacles += [info["bbox"] for info in page.get_image_info()]
        except Exception:
            pass

        for bbox in obstacles:
            start, _end = across_span(rotation, bbox)
            if start <= bottom + 0.5:
                continue  # level with the paragraph or before it
            a0, a1 = along_span(rotation, bbox)
            if a1 <= near + 1 or a0 >= far - 1:
                continue  # beside it, in another column
            floor = min(floor, start)
        return floor

    # Items a moved drawing may be made of. Anything else — a clip, a shading —
    # is left alone by refusing to move the block at all, because half a figure
    # in its old place is worse than a paragraph that overflows.
    _MOVABLE_ITEMS = {"l", "re", "c", "qu"}

    def _push_down(
        self,
        pno: int,
        rotation: int,
        boxes: list[pymupdf.Rect],
        delta: float,
        batch: _PageBatch,
    ) -> bool:
        """Move whatever sits below the paragraph out of its way.

        Returns False, changing nothing, when the block below cannot be moved
        faithfully: a figure, an image, or anything that would be pushed off the
        sheet. Refusing is the right answer there — the caller falls back to
        resizing the text or to saying it does not fit.
        """
        page = self._doc[pno]
        bottom = max(across_span(rotation, box)[1] for box in boxes)
        near = min(along_span(rotation, box)[0] for box in boxes)
        far = max(along_span(rotation, box)[1] for box in boxes)
        page_limit = max(
            across(rotation, x, y)
            for x, y in ((page.rect.x0, page.rect.y0), (page.rect.x1, page.rect.y1))
        )

        def below(bbox) -> bool:
            start, _end = across_span(rotation, bbox)
            if start <= bottom + 0.5:
                return False
            a0, a1 = along_span(rotation, bbox)
            return a1 > near + 1 and a0 < far - 1

        moving_images: list[tuple[pymupdf.Rect, bytes]] = []
        if page.get_images():
            if was_recognised(page):
                # The page is one big picture with text read off it. There is
                # nothing below to move that is not the page itself.
                return False
            for info in page.get_image_info(xrefs=True):
                if not below(info["bbox"]):
                    continue
                data = self._image_bytes(info.get("xref", 0))
                if data is None:
                    return False  # cannot move it faithfully
                moving_images.append((pymupdf.Rect(info["bbox"]), data))

        moving_drawings = []
        for drawing in page.get_drawings():
            if not below(drawing["rect"]):
                continue
            if any(item[0] not in self._MOVABLE_ITEMS for item in drawing["items"]):
                return False
            moving_drawings.append(drawing)

        try:
            raw = page.get_text("dict")
        except Exception:
            return False

        moving_lines = [
            line
            for block in raw.get("blocks", [])
            if block.get("type") == 0
            for line in block.get("lines", [])
            if below(line.get("bbox", (0, 0, 0, 0)))
        ]

        if not moving_lines and not moving_drawings and not moving_images:
            return True

        # Would any of it fall off the sheet? What does is carried over to a
        # page of its own rather than pushed past the edge.
        floor = page_limit - 18
        pieces = (
            [(across_span(rotation, line["bbox"]), ("line", line)) for line in moving_lines]
            + [(across_span(rotation, d["rect"]), ("drawing", d)) for d in moving_drawings]
            + [(across_span(rotation, rect), ("image", (rect, data)))
               for rect, data in moving_images]
        )
        overflowing = [piece for span, piece in pieces if span[1] + delta > floor]
        if overflowing:
            if not self._carry_over(pno, rotation, overflowing, batch):
                return False
            page = self._doc[pno]  # a page was inserted; the old handle is stale
            carried = {id(piece[1]) for piece in overflowing}
            moving_lines = [x for x in moving_lines if id(x) not in carried]
            moving_drawings = [x for x in moving_drawings if id(x) not in carried]
            moving_images = [
                (rect, data) for rect, data in moving_images
                if not any(p[0] == "image" and p[1][0] is rect for p in overflowing)
            ]

        shift = pymupdf.Point(*from_axes(rotation, 0.0, delta))
        batch.remove_line_art = True
        if moving_images:
            batch.move_images = True
            for rect, data in moving_images:
                batch.redactions.append(rect)
                batch.moved_image_sources.add(tuple(round(v, 1) for v in rect))
                batch.restored_images.append(
                    (rect + (shift.x, shift.y, shift.x, shift.y), data)
                )

        for line in moving_lines:
            batch.redactions.append(
                pymupdf.Rect(line["bbox"])
                + (-_REDACT_PAD, -_REDACT_PAD, _REDACT_PAD, _REDACT_PAD)
            )
            for span in line.get("spans", []):
                text = span.get("text", "")
                if not text.strip():
                    continue
                descriptor = {
                    "text": text,
                    "font": span.get("font"),
                    "size": float(span.get("size", 11.0)),
                    "flags": int(span.get("flags", 0)),
                }
                origin = span.get("origin", (0, 0))
                batch.texts.append(
                    _PendingText(
                        page=pno,
                        point=(origin[0] + shift.x, origin[1] + shift.y),
                        text=text,
                        resolved=_span_font(self._resolver, pno, descriptor),
                        fontsize=descriptor["size"],
                        color=pymupdf.sRGB_to_pdf(int(span.get("color", 0))),
                        rotate=rotation,
                        opacity=_visible_opacity(span.get("alpha", 255)),
                    )
                )

        for drawing in moving_drawings:
            # A drawing's box is its path, and a stroke straddles that path:
            # half the pen's width spills past it on every side. A rectangle
            # that only covers the path does not contain the drawing, and line
            # art is only erased when it is contained — so the original would
            # survive beside the copy.
            pad = _REDACT_PAD + (drawing.get("width") or 0)
            batch.redactions.append(drawing["rect"] + (-pad, -pad, pad, pad))
            batch.drawings.append((drawing, shift))

        for link in page.get_links():
            if not below(link["from"]):
                continue
            if tuple(round(v, 1) for v in link["from"]) in batch.carried_links:
                continue  # it already went to the continuation page
            moved = dict(link)
            moved["from"] = pymupdf.Rect(link["from"]) + (shift.x, shift.y, shift.x, shift.y)
            batch.new_links.append(moved)

        return True

    def _carry_over(
        self,
        pno: int,
        rotation: int,
        pieces: list[tuple[str, Any]],
        batch: _PageBatch,
    ) -> bool:
        """Move what no longer fits onto a page of its own, right after this one.

        Pushing content past the foot of the sheet would simply hide it. A
        continuation page keeps it readable and keeps the order it was in.
        Attempted once: if it does not fit there either, the caller is told no
        rather than handed a chain of half-filled pages.
        """
        if len(self._batches) > 1:
            # Page numbers after this one are about to shift; another page's
            # operations in the same batch would end up pointing at the wrong
            # page.
            return False

        source = self._doc[pno]
        starts = [across_span(rotation, self._rect_of(piece))[0] for piece in pieces]
        ends = [across_span(rotation, self._rect_of(piece))[1] for piece in pieces]
        first = min(starts)
        page_limit = max(
            across(rotation, x, y)
            for x, y in ((source.rect.x0, source.rect.y0), (source.rect.x1, source.rect.y1))
        )
        top = self._top_margin(source, rotation)
        if (max(ends) - first) > (page_limit - 18) - top:
            return False  # taller than a page: nothing to be done here

        rotation_of_page = source.rotation
        self._doc.new_page(pno=pno + 1, width=source.rect.width, height=source.rect.height)
        # Inserting a page invalidates the page objects already in hand.
        source = self._doc[pno]
        fresh = self._doc[pno + 1]
        if rotation_of_page:
            fresh.set_rotation(rotation_of_page)
        carried = self._batch(pno + 1)

        lift = top - first
        shift = pymupdf.Point(*from_axes(rotation, 0.0, lift))

        for kind, item in pieces:
            if kind == "line":
                batch.redactions.append(
                    pymupdf.Rect(item["bbox"])
                    + (-_REDACT_PAD, -_REDACT_PAD, _REDACT_PAD, _REDACT_PAD)
                )
                for span in item.get("spans", []):
                    if not span.get("text", "").strip():
                        continue
                    descriptor = {
                        "text": span["text"],
                        "font": span.get("font"),
                        "size": float(span.get("size", 11.0)),
                        "flags": int(span.get("flags", 0)),
                    }
                    origin = span.get("origin", (0, 0))
                    carried.texts.append(
                        _PendingText(
                            page=pno + 1,
                            point=(origin[0] + shift.x, origin[1] + shift.y),
                            text=span["text"],
                            resolved=_span_font(self._resolver, pno, descriptor),
                            fontsize=descriptor["size"],
                            color=pymupdf.sRGB_to_pdf(int(span.get("color", 0))),
                            rotate=rotation,
                            opacity=_visible_opacity(span.get("alpha", 255)),
                        )
                    )
            elif kind == "drawing":
                pad = _REDACT_PAD + (item.get("width") or 0)
                batch.redactions.append(item["rect"] + (-pad, -pad, pad, pad))
                batch.remove_line_art = True
                carried.drawings.append((item, shift))
            else:
                rect, data = item
                batch.redactions.append(rect)
                batch.move_images = True
                batch.moved_image_sources.add(tuple(round(v, 1) for v in rect))
                carried.restored_images.append(
                    (rect + (shift.x, shift.y, shift.x, shift.y), data)
                )

        boxes = [self._rect_of(piece) for piece in pieces]
        for link in source.get_links():
            rect = pymupdf.Rect(link["from"])
            if not any(rect.intersects(box) for box in boxes):
                continue
            moved = dict(link)
            moved["from"] = rect + (shift.x, shift.y, shift.x, shift.y)
            carried.new_links.append(moved)
            batch.carried_links.add(tuple(round(v, 1) for v in rect))
        return True

    @staticmethod
    def _rect_of(piece: tuple[str, Any]) -> pymupdf.Rect:
        """The box of whatever kind of thing a carried piece is."""
        kind, item = piece
        if kind == "line":
            return pymupdf.Rect(item["bbox"])
        if kind == "drawing":
            return pymupdf.Rect(item["rect"])
        return pymupdf.Rect(item[0])

    def _top_margin(self, page: pymupdf.Page, rotation: int) -> float:
        """Where a continuation page should start putting things.

        Taken from the whole document rather than from the page being
        continued: a page whose content happens to sit at its foot says nothing
        about where a fresh one should begin. Falls back to a plain margin when
        the document offers no answer.
        """
        edges = [
            across(rotation, x, y)
            for x, y in ((page.rect.x0, page.rect.y0), (page.rect.x1, page.rect.y1))
        ]
        edge = min(edges)
        starts = []
        for pno in range(self._doc.page_count):
            try:
                raw = self._doc[pno].get_text("dict")
            except Exception:
                continue
            for block in raw.get("blocks", []):
                for line in block.get("lines", []) if block.get("type") == 0 else []:
                    starts.append(across_span(rotation, line.get("bbox", (0, 0, 0, 0)))[0])
        if not starts:
            return edge + 56
        # Never closer to the edge than a thumb's width, never further down
        # than a third of the sheet: both would look like a mistake.
        span = max(e for e in edges) - edge
        return min(max(min(starts), edge + 36), edge + span / 3)

    def _image_bytes(self, xref: int) -> bytes | None:
        """The picture behind an xref, exactly as the file stores it."""
        if not xref:
            return None
        try:
            return self._doc.extract_image(xref)["image"] or None
        except Exception:
            return None

    def _reflow(
        self,
        pno: int,
        lines: list[dict[str, Any]],
        box: pymupdf.Rect,
        align: str,
        fit: str,
        measure_point: Sequence[float] | None = None,
    ) -> None:
        """Re-break a paragraph and lay it out again from its first baseline."""
        batch = self._batch(pno)
        rotation = self._rotation(pno, lines[0].get("rotation", 0))
        runs, tokens = self._paragraph_stream(pno, lines, rotation)
        boxes = [self._rect(pno, line["bbox"]) for line in lines]
        if not tokens:
            for rect in boxes:
                batch.redactions.append(
                    rect + (-_REDACT_PAD, -_REDACT_PAD, _REDACT_PAD, _REDACT_PAD)
                )
            return

        origins = [
            self._point(pno, line.get("origin") or [rect.x0, rect.y1])
            for line, rect in zip(lines, boxes)
        ]
        starts = [along_span(rotation, rect)[0] for rect in boxes]
        first_start, body_start = starts[0], (starts[1] if len(starts) > 1 else starts[0])
        limit = (
            along(rotation, *self._point(pno, measure_point))
            if measure_point
            else along_span(rotation, box)[1]
        )
        base_across = across(rotation, *origins[0])
        leading = _median_leading(origins, boxes, rotation)

        # The paragraph's own spacing is the right starting point — it may be
        # deliberately loose — but never less than the text itself needs, or the
        # lines run into each other. What it needs comes from the fonts being
        # drawn, not from the sizes the request arrived with: those already
        # carry any change the user just made.
        natural = max(
            ((run.font.font.ascender - run.font.font.descender) * run.size for run in runs),
            default=0.0,
        )
        leading = max(leading, natural)

        descent = max((-run.font.font.descender * run.size for run in runs), default=0.0)
        ceiling = self._room_below(pno, boxes, rotation) - descent
        natural_leading = leading

        def reach(count: int, spacing: float) -> float:
            return base_across + max(count - 1, 0) * spacing

        scale = 1.0
        for _ in range(10):
            wrapped = _wrap(tokens, limit - first_start, limit - body_start)
            if reach(len(wrapped), leading) <= ceiling + 0.5:
                break

            # First give up some line spacing: much less visible than resizing.
            if len(wrapped) > 1:
                tight = (ceiling - base_across) / (len(wrapped) - 1)
                if tight >= natural_leading * _MIN_LEADING:
                    leading = tight
                    break

            # Then, if asked, move what is in the way instead of shrinking.
            if fit == "push":
                needed = reach(len(wrapped), leading) - ceiling
                if self._push_down(pno, rotation, boxes, needed, batch):
                    ceiling += needed
                    break
                self._warn(
                    pno,
                    "cannot-push",
                    "No se puede desplazar lo que hay debajo sin romperlo; "
                    "prueba con «ajustar el tamaño».",
                )
                break

            if fit != "shrink" or scale <= _MIN_SHRINK:
                break

            # Then the text itself.
            scale = max(scale * 0.94, _MIN_SHRINK)
            for run in runs:
                run.size = float(run.size) * 0.94
            for token in tokens:
                token.width = runs[token.run].font.text_length(
                    token.text, runs[token.run].size
                )
            natural_leading *= 0.94
            leading = natural_leading

        if reach(len(wrapped), leading) > ceiling + 0.5:
            self._warn(
                pno,
                "paragraph-grew",
                f"El párrafo pasa de {len(lines)} a {len(wrapped)} líneas y no cabe "
                "en el hueco disponible; usa «Ajustar» o acorta el texto.",
            )

        for rect in boxes:
            batch.redactions.append(rect + (-_REDACT_PAD, -_REDACT_PAD, _REDACT_PAD, _REDACT_PAD))

        for number, wrapped_line in enumerate(wrapped):
            start = first_start if number == 0 else body_start
            total = sum(token.width for token in wrapped_line)
            line_across = base_across + number * leading
            cursor = start + _start_offset(align, limit - start, total)

            for token in wrapped_line:
                token.placed_line = number
                token.placed_from = cursor
                token.placed_to = cursor + token.width
                cursor += token.width

            cursor = start + _start_offset(align, limit - start, total)
            for run_index, text in _group_into_runs(wrapped_line):
                run = runs[run_index]
                batch.texts.append(
                    _PendingText(
                        page=pno,
                        point=from_axes(rotation, cursor, line_across),
                        text=text,
                        resolved=run.font,
                        fontsize=run.size,
                        color=run.color,
                        rotate=rotation,
                        opacity=run.opacity,
                    )
                )
                cursor += run.font.text_length(text, run.size)

        self._relocate_links(pno, boxes, base_across, tokens, wrapped, leading, rotation, batch)

    def _relocate_links(
        self,
        pno: int,
        boxes: list[pymupdf.Rect],
        base_across: float,
        tokens: list[_Token],
        wrapped: list[list[_Token]],
        leading: float,
        rotation: int,
        batch: _PageBatch,
    ) -> None:
        """Move the paragraph's links onto the words they used to sit over."""
        page = self._doc[pno]
        spans = [(along_span(rotation, box), across_span(rotation, box)) for box in boxes]
        thickness = spans[0][1][1] - spans[0][1][0]
        offset = spans[0][1][1] - base_across  # baseline to the bottom of the line

        for link in page.get_links():
            rect = pymupdf.Rect(link["from"])
            link_along = along_span(rotation, rect)
            link_across = across_span(rotation, rect)
            covered = [
                token
                for token in tokens
                if token.placed_line >= 0
                and 0 <= token.source_line < len(spans)
                and link_across[0] < spans[token.source_line][1][1]
                and link_across[1] > spans[token.source_line][1][0]
                and token.source_to > link_along[0]
                and token.source_from < link_along[1]
            ]
            if not covered:
                continue

            by_line: dict[int, list[_Token]] = {}
            for token in covered:
                by_line.setdefault(token.placed_line, []).append(token)

            for number, group in by_line.items():
                bottom = base_across + number * leading + offset
                corners = [
                    from_axes(rotation, min(t.placed_from for t in group), bottom - thickness),
                    from_axes(rotation, max(t.placed_to for t in group), bottom),
                ]
                moved = dict(link)
                moved["from"] = pymupdf.Rect(
                    min(corners[0][0], corners[1][0]), min(corners[0][1], corners[1][1]),
                    max(corners[0][0], corners[1][0]), max(corners[0][1], corners[1][1]),
                )
                batch.new_links.append(moved)

    def move_block(self, op: dict[str, Any]) -> None:
        """Pick a run of text up and set it down somewhere else on the page.

        Only the text moves, with whatever links sit on it. A drawing or a
        picture the block happens to overlap belongs to the page, not to the
        words, and stays where the page put it.
        """
        pno = int(op["page"])
        batch = self._batch(pno)
        lines = op.get("lines") or []
        if not lines:
            raise EditError("No hay nada que mover")

        page = self._doc[pno]
        shift = pymupdf.Point(float(op.get("dx", 0.0)), float(op.get("dy", 0.0)))
        if page.rotation:
            # The client drags in the space it sees; the page is drawn in its own.
            origin = to_page(page, pymupdf.Point(0, 0))
            moved = to_page(page, pymupdf.Point(shift.x, shift.y))
            shift = pymupdf.Point(moved.x - origin.x, moved.y - origin.y)

        if abs(shift.x) < 0.5 and abs(shift.y) < 0.5:
            return  # a click, not a drag

        boxes = [self._rect(pno, line["bbox"]) for line in lines]
        landing = [rect + (shift.x, shift.y, shift.x, shift.y) for rect in boxes]
        if not all(page.rect.contains(rect) for rect in landing):
            raise EditError("El texto no cabe ahí; quedaría fuera de la página.")

        for line, rect in zip(lines, boxes):
            batch.redactions.append(
                rect + (-_REDACT_PAD, -_REDACT_PAD, _REDACT_PAD, _REDACT_PAD)
            )
            rotation = self._rotation(pno, line.get("rotation", 0))
            for span in line.get("spans", []):
                text = span.get("text", "")
                if not text.strip():
                    continue
                font = _span_font(self._resolver, pno, span)
                if font.note:
                    self._warn(pno, "font-substituted", font.note)
                start = self._point(pno, span.get("origin") or [rect.x0, rect.y1])
                batch.texts.append(
                    _PendingText(
                        page=pno,
                        point=(start[0] + shift.x, start[1] + shift.y),
                        text=text,
                        resolved=font,
                        fontsize=float(span.get("size", 11.0)),
                        color=hex_to_pdf(span.get("color", "#000000")),
                        rotate=rotation,
                        opacity=_visible_opacity(span.get("alpha", 255)),
                    )
                )

        for link in page.get_links():
            rect = pymupdf.Rect(link["from"])
            if not any(rect.intersects(box) for box in boxes):
                continue
            moved = dict(link)
            moved["from"] = rect + (shift.x, shift.y, shift.x, shift.y)
            batch.new_links.append(moved)

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
            links_before = page.get_links() if batch.redactions else []
            if batch.redactions:
                if batch.remove_line_art:
                    # Erasing line art is all or nothing over the page's
                    # redactions, so anything caught that is not being moved has
                    # to be put back exactly where it was. Taken before the
                    # annotations go on, because their own marks would otherwise
                    # be read as drawings of the page.
                    self._preserve_untouched_drawings(page, batch)
                if batch.move_images:
                    self._preserve_untouched_images(page, batch)
                for rect in batch.redactions:
                    if not rect.is_empty:
                        page.add_redact_annot(rect, cross_out=False)
                page.apply_redactions(
                    # On a recognised scan the old words are pixels, not text.
                    # Erasing the text layer alone would leave them showing
                    # through whatever is written in their place.
                    images=self._image_mode(page, batch),
                    graphics=(
                        # Only "if touched" actually erases: a rectangle that
                        # contains a curve's box is not enough for "if covered",
                        # which would leave the original beside its copy.
                        # Everything it takes is put back, moved or not.
                        pymupdf.PDF_REDACT_LINE_ART_REMOVE_IF_TOUCHED
                        if batch.remove_line_art
                        else pymupdf.PDF_REDACT_LINE_ART_NONE
                    ),
                    text=pymupdf.PDF_REDACT_TEXT_REMOVE,
                )
                # The content stream was rewritten; page font resources with it.
                self._resolver.forget_page(pno)

            for pending in batch.texts:
                fontname = self._resolver.install(page, pending.resolved)
                # A stand-in font is rarely the width of the one it replaces, so
                # it is squeezed to the width the original occupied. Without
                # this the replacement runs long and pushes the line's own
                # wrapping about, even when nothing was edited.
                hscale = pending.resolved.hscale
                morph = (
                    (pymupdf.Point(*pending.point), _scale_matrix(hscale, pending.rotate))
                    if abs(hscale - 1.0) > 0.001
                    else None
                )
                try:
                    page.insert_text(
                        pending.point,
                        pending.text,
                        fontname=fontname,
                        fontsize=pending.fontsize,
                        color=pending.color,
                        rotate=pending.rotate,
                        fill_opacity=pending.opacity,
                        morph=morph,
                        overlay=True,
                    )
                except Exception as exc:  # pragma: no cover - defensive
                    raise EditError(f"No se pudo escribir el texto: {exc}") from exc

            for rect, data in batch.restored_images:
                try:
                    page.insert_image(rect, stream=data)
                except Exception:
                    continue

            for drawing, shift in batch.drawings:
                _redraw(page, drawing, shift)

            for rect, data, rotate in batch.images:
                try:
                    page.insert_image(rect, stream=data, rotate=rotate, keep_proportion=True)
                except Exception as exc:
                    raise EditError(f"No se pudo insertar la imagen: {exc}") from exc

            for link in batch.new_links:
                try:
                    page.insert_link(link)
                except Exception:
                    continue
            if links_before and not batch.new_links:
                self._restore_links(page, links_before, batch)

        return self.warnings

    @staticmethod
    def _image_mode(page: pymupdf.Page, batch: _PageBatch) -> int:
        """How redaction should treat the pictures it runs into.

        Removed when one has to move, because moving means taking it out and
        putting it back. Blanked on a recognised scan, where the old words are
        pixels that would otherwise show through. Left alone otherwise.
        """
        if batch.move_images:
            return pymupdf.PDF_REDACT_IMAGE_REMOVE
        if was_recognised(page):
            return pymupdf.PDF_REDACT_IMAGE_PIXELS
        return pymupdf.PDF_REDACT_IMAGE_NONE

    def _preserve_untouched_images(self, page: pymupdf.Page, batch: _PageBatch) -> None:
        """Queue every picture a redaction would remove but nothing asked to move."""
        for info in page.get_image_info(xrefs=True):
            rect = pymupdf.Rect(info["bbox"])
            if tuple(round(v, 1) for v in rect) in batch.moved_image_sources:
                continue  # already queued, at its new place
            if not any(r.intersects(rect) for r in batch.redactions):
                continue
            data = self._image_bytes(info.get("xref", 0))
            if data is not None:
                batch.restored_images.append((rect, data))

    @staticmethod
    def _preserve_untouched_drawings(page: pymupdf.Page, batch: _PageBatch) -> None:
        """Queue every drawing a redaction would take but nothing asked to move."""
        moving = {id(drawing) for drawing, _shift in batch.drawings}
        marked = [
            (tuple(round(v, 2) for v in drawing["rect"]), drawing)
            for drawing, _shift in batch.drawings
        ]
        moving_rects = {key for key, _drawing in marked}
        nothing = pymupdf.Point(0, 0)
        for drawing in page.get_drawings():
            key = tuple(round(v, 2) for v in drawing["rect"])
            if id(drawing) in moving or key in moving_rects:
                continue
            # Matched to how the erasing works: anything a redaction touches
            # goes, so anything it touches has to come back.
            if any(rect.intersects(drawing["rect"]) for rect in batch.redactions):
                batch.drawings.append((drawing, nothing))

    @staticmethod
    def _restore_links(
        page: pymupdf.Page, before: list[dict[str, Any]], batch: _PageBatch
    ) -> None:
        """Put back the links redaction removed, over the text that replaced them.

        A link whose line was rewritten moves with the new text. One whose text
        was deleted outright is not restored: there is nothing left to click.
        """
        surviving = {
            (link.get("uri"), tuple(round(v, 1) for v in link["from"]))
            for link in page.get_links()
        }
        for link in before:
            rect = pymupdf.Rect(link["from"])
            key = (link.get("uri"), tuple(round(v, 1) for v in link["from"]))
            if key in surviving:
                continue
            rewrite = next(
                (r for r in batch.rewrites if rect.intersects(r.old) and not r.new.is_empty),
                None,
            )
            if rewrite is None:
                continue
            restored = dict(link)
            restored["from"] = _remap(rect, rewrite)
            try:
                page.insert_link(restored)
            except Exception:
                continue  # a link type this page can no longer carry


# Operations that change which pages exist, or in what order. A step that
# includes one cannot be recorded page by page.
_STRUCTURAL = {"delete_page", "move_page", "insert_page"}


def pages_touched(operations: Iterable[dict[str, Any]]) -> list[int] | None:
    """Which pages a batch will change, or None when it changes the structure.

    Used to record an undo step of just those pages. Erring towards None costs
    memory; erring the other way would lose a page's previous state, so
    anything not plainly page-local counts as structural.
    """
    pages: set[int] = set()
    for op in operations:
        kind = op.get("op")
        if kind in _STRUCTURAL:
            return None
        page = op.get("page")
        if page is None:
            return None
        pages.add(int(page))
        if kind == "rotate_page":
            pages.add(int(page))
    return sorted(pages)


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
        elif kind == "replace_paragraph":
            session.replace_paragraph(op)
        elif kind == "move_block":
            session.move_block(op)
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
