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

from .extract import hex_to_pdf, page_rotation_of, to_page
from .fonts import FLAG_BOLD, FLAG_ITALIC, FontResolver, ResolvedFont

# Redaction rectangles are grown by this much so no anti-aliased sliver of the
# old glyphs survives, and no more, so neighbouring lines are left alone.
_REDACT_PAD = 0.4

# A replacement is never shrunk past this fraction of its original size; below
# it the result looks wrong enough that overflowing is the better answer.
_MIN_SHRINK = 0.5


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


def _wrap(tokens: list[_Token], first_width: float, width: float) -> list[list[_Token]]:
    """Greedily break tokens into lines, the way a text engine does.

    The first line may be narrower than the rest, because a paragraph's opening
    line is often indented.
    """
    lines: list[list[_Token]] = []
    current: list[_Token] = []
    used = 0.0
    limit = max(first_width, 1.0)

    for token in tokens:
        if token.is_space and not current:
            continue  # a wrapped line does not start with a space
        if not token.is_space and current and used + token.width > limit:
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


def _median_leading(
    origins: list[tuple[float, float]], boxes: list[pymupdf.Rect]
) -> float:
    """The paragraph's own line spacing, read from its baselines."""
    deltas = sorted(
        b[1] - a[1] for a, b in zip(origins, origins[1:]) if b[1] - a[1] > 0
    )
    if deltas:
        return deltas[len(deltas) // 2]
    return max((box.y1 - box.y0 for box in boxes), default=12.0) * 1.2


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
                    opacity=float(span.get("alpha", 255)) / 255.0,
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

        rotated = any(int(line.get("rotation", 0)) % 360 for line in lines)
        single = len(lines) == 1
        forced = bool(op.get("reflow"))

        if not forced and (rotated or single or self._line_still_fits(pno, lines[index], box, align)):
            # Nothing has to move but the line being edited.
            self.replace_line({**lines[index], "op": "replace_line", "page": pno,
                               "align": align, "fit": op.get("fit", "overflow"),
                               "from_span": edited.get("span", 0)})
            return

        if rotated:
            self._warn(pno, "no-reflow", "El texto girado no se reajusta entre líneas.")
            self.replace_line({**lines[index], "op": "replace_line", "page": pno, "align": align})
            return

        self._reflow(pno, lines, box, align, op.get("fit", "overflow"))

    def _line_still_fits(
        self, pno: int, line: dict[str, Any], box: pymupdf.Rect, align: str
    ) -> bool:
        """Whether the edited line stays inside the paragraph's right margin."""
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
        start = self._rect(pno, line["bbox"]).x0
        return needed <= (box.x1 - start) + 0.5

    def _paragraph_stream(
        self, pno: int, lines: list[dict[str, Any]]
    ) -> tuple[list[_Run], list[_Token]]:
        """Flatten a paragraph into styled runs and the words that make it up.

        Each word keeps where it was on the page, so anything anchored to it —
        a hyperlink — can be found again after the paragraph is re-broken.
        """
        runs: list[_Run] = []
        tokens: list[_Token] = []

        for position, line in enumerate(lines):
            cursor = self._rect(pno, line["bbox"]).x0
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
                        opacity=float(span.get("alpha", 255)) / 255.0,
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

    def _reflow(
        self,
        pno: int,
        lines: list[dict[str, Any]],
        box: pymupdf.Rect,
        align: str,
        fit: str,
    ) -> None:
        """Re-break a paragraph and lay it out again from its first baseline."""
        batch = self._batch(pno)
        runs, tokens = self._paragraph_stream(pno, lines)
        if not tokens:
            for line in lines:
                batch.redactions.append(
                    self._rect(pno, line["bbox"])
                    + (-_REDACT_PAD, -_REDACT_PAD, _REDACT_PAD, _REDACT_PAD)
                )
            return

        boxes = [self._rect(pno, line["bbox"]) for line in lines]
        origins = [self._point(pno, line.get("origin") or [b.x0, b.y1]) for line, b in zip(lines, boxes)]
        first_left = boxes[0].x0
        body_left = boxes[1].x0 if len(boxes) > 1 else first_left
        leading = _median_leading(origins, boxes)

        # Bigger text needs more room between baselines. The paragraph's own
        # spacing is the right starting point — it may be deliberately loose —
        # but it cannot be less than the text itself needs, or the lines run
        # into each other. What it needs comes from the fonts being drawn, not
        # from the sizes the request arrived with: those already carry any
        # change the user just made.
        natural = max(
            (
                (run.font.font.ascender - run.font.font.descender) * run.size
                for run in runs
            ),
            default=0.0,
        )
        leading = max(leading, natural)

        scale = 1.0
        for _ in range(8):
            wrapped = _wrap(tokens, box.x1 - first_left, box.x1 - body_left)
            if fit != "shrink" or len(wrapped) <= len(lines) or scale <= _MIN_SHRINK:
                break
            scale = max(scale * 0.94, _MIN_SHRINK)
            for run in runs:
                run.size = float(run.size) * 0.94
            for token in tokens:
                token.width = runs[token.run].font.text_length(
                    token.text, runs[token.run].size
                )
            leading *= 0.94
        else:
            wrapped = _wrap(tokens, box.x1 - first_left, box.x1 - body_left)

        if len(wrapped) > len(lines):
            self._warn(
                pno,
                "paragraph-grew",
                f"El párrafo pasa de {len(lines)} a {len(wrapped)} líneas y puede "
                "solaparse con lo que viene debajo.",
            )

        for rect in boxes:
            batch.redactions.append(rect + (-_REDACT_PAD, -_REDACT_PAD, _REDACT_PAD, _REDACT_PAD))

        baseline = origins[0][1]
        for number, wrapped_line in enumerate(wrapped):
            left = first_left if number == 0 else body_left
            total = sum(token.width for token in wrapped_line)
            cursor = left + _start_offset(align, box.x1 - left, total)
            for token in wrapped_line:
                token.placed_line = number
                token.placed_from = cursor
                token.placed_to = cursor + token.width
                cursor += token.width

            cursor = left + _start_offset(align, box.x1 - left, total)
            for run_index, text in _group_into_runs(wrapped_line):
                run = runs[run_index]
                batch.texts.append(
                    _PendingText(
                        page=pno, point=(cursor, baseline), text=text,
                        resolved=run.font, fontsize=run.size, color=run.color,
                        rotate=0, opacity=run.opacity,
                    )
                )
                cursor += run.font.text_length(text, run.size)
            baseline += leading

        self._relocate_links(pno, boxes, origins, tokens, wrapped, leading, batch)

    def _relocate_links(
        self,
        pno: int,
        boxes: list[pymupdf.Rect],
        origins: list[tuple[float, float]],
        tokens: list[_Token],
        wrapped: list[list[_Token]],
        leading: float,
        batch: _PageBatch,
    ) -> None:
        """Move the paragraph's links onto the words they used to sit over."""
        page = self._doc[pno]
        for link in page.get_links():
            rect = pymupdf.Rect(link["from"])
            covered = [
                token
                for token in tokens
                if token.placed_line >= 0
                and 0 <= token.source_line < len(boxes)
                and rect.y0 < boxes[token.source_line].y1
                and rect.y1 > boxes[token.source_line].y0
                and token.source_to > rect.x0
                and token.source_from < rect.x1
            ]
            if not covered:
                continue
            by_line: dict[int, list[_Token]] = {}
            for token in covered:
                by_line.setdefault(token.placed_line, []).append(token)
            height = boxes[0].y1 - boxes[0].y0
            for number, group in by_line.items():
                top = origins[0][1] + number * leading - height + (boxes[0].y1 - origins[0][1])
                moved = dict(link)
                moved["from"] = pymupdf.Rect(
                    min(t.placed_from for t in group), top,
                    max(t.placed_to for t in group), top + height,
                )
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

            for link in batch.new_links:
                try:
                    page.insert_link(link)
                except Exception:
                    continue
            if links_before and not batch.new_links:
                self._restore_links(page, links_before, batch)

        return self.warnings

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
