"""Reading a page's text back out as editable pieces.

The editor works on *lines*: a line is the smallest run of text that can be
re-laid-out on its own, and it owns the spans (uniformly formatted runs) the
user actually types into. Every geometric fact the editor needs later to put
the text back — baseline origin, writing direction, alignment within its
paragraph — is captured here, because once a line is redacted it is gone.
"""

from __future__ import annotations

import math
import re
from typing import Any, Iterable, Sequence

import pymupdf

from .fonts import FontResolver, style_of

# Two edges count as aligned when they sit within this many points of each
# other; PDF text rarely lines up to the micro-point.
_ALIGN_TOLERANCE = 1.5


def rotation_from_dir(direction: Iterable[float]) -> int:
    """Turn a line's direction cosine into degrees of rotation."""
    dx, dy = list(direction)[:2]
    if math.isclose(dx, 1, abs_tol=0.01):
        return 0
    if math.isclose(dx, -1, abs_tol=0.01):
        return 180
    if math.isclose(dy, -1, abs_tol=0.01):
        return 90
    if math.isclose(dy, 1, abs_tol=0.01):
        return 270
    return 0


def color_to_hex(color: int) -> str:
    """sRGB integer from extraction to ``#rrggbb``."""
    return f"#{color & 0xFFFFFF:06x}"


def hex_to_pdf(value: str) -> tuple[float, float, float]:
    """``#rrggbb`` to the 0..1 RGB triple PyMuPDF draws with."""
    text = (value or "#000000").lstrip("#")
    if len(text) == 3:
        text = "".join(ch * 2 for ch in text)
    try:
        number = int(text, 16)
    except ValueError:
        number = 0
    return pymupdf.sRGB_to_pdf(number)


def pdf_to_hex(triple: Sequence[float] | None) -> str | None:
    """The 0..1 RGB triple PyMuPDF reports back to ``#rrggbb``."""
    if not triple or len(triple) < 3:
        return None
    return "#" + "".join(f"{max(0, min(255, round(v * 255))):02x}" for v in triple[:3])


def to_display(page: pymupdf.Page, rect_or_point):
    """Map page coordinates into the space the rendered image uses.

    ``get_text`` reports coordinates ignoring the page's ``/Rotate`` entry, but
    ``get_pixmap`` honours it, so on a rotated page the two disagree. Everything
    leaving the server is mapped into the rendered space, and everything coming
    back is mapped out of it, so the client only ever sees what it can see.
    """
    return rect_or_point * page.rotation_matrix


def to_page(page: pymupdf.Page, rect_or_point):
    """Inverse of :func:`to_display`."""
    return rect_or_point * page.derotation_matrix


def display_rotation(page_rotation: int, line_rotation: int) -> int:
    """Writing direction as it appears on the rendered page."""
    return (line_rotation + page_rotation) % 360


def page_rotation_of(page_rotation: int, display_line_rotation: int) -> int:
    """Inverse of :func:`display_rotation`."""
    return (display_line_rotation - page_rotation) % 360


# Text does not always run left to right. Rather than special-casing each
# quarter turn everywhere, a line's geometry is expressed in two coordinates
# tied to its own direction: `along`, which grows the way the text reads, and
# `across`, which grows the way the next line lies. For unrotated text they are
# plain x and y, and everything below reads as it always did.


def along(rotation: int, x: float, y: float) -> float:
    """Position in the direction the text reads."""
    return {0: x, 90: -y, 180: -x, 270: y}[rotation % 360]


def across(rotation: int, x: float, y: float) -> float:
    """Position in the direction successive lines lie."""
    return {0: y, 90: x, 180: -y, 270: -x}[rotation % 360]


def from_axes(rotation: int, a: float, c: float) -> tuple[float, float]:
    """Back from ``(along, across)`` to a point on the page."""
    return {
        0: (a, c),
        90: (c, -a),
        180: (-a, -c),
        270: (-c, a),
    }[rotation % 360]


def along_span(rotation: int, bbox: Sequence[float]) -> tuple[float, float]:
    """Where a box starts and ends in the reading direction."""
    x0, y0, x1, y1 = (float(v) for v in bbox)
    first, last = along(rotation, x0, y0), along(rotation, x1, y1)
    return (min(first, last), max(first, last))


def across_span(rotation: int, bbox: Sequence[float]) -> tuple[float, float]:
    """Where a box starts and ends across the lines."""
    x0, y0, x1, y1 = (float(v) for v in bbox)
    first, last = across(rotation, x0, y0), across(rotation, x1, y1)
    return (min(first, last), max(first, last))


def _cluster_lines(lines: list[dict]) -> list[list[dict]]:
    """Group lines that read as one paragraph.

    Alignment can only be seen across several lines, and PyMuPDF's own blocks
    are not a reliable paragraph: how a PDF's content stream happens to be
    written decides them, and plenty of files give every line a block of its
    own. Vertical adjacency plus horizontal overlap is what a reader actually
    uses, so that is what is used here.
    """
    clusters: list[list[dict]] = []
    for rotation, group in _by_rotation(lines):
        for line in group:
            placed = False
            if clusters and clusters[-1][-1]["rotation"] == rotation:
                previous = clusters[-1][-1]
                low, high = across_span(rotation, previous["bbox"])
                height = max(high - low, 1.0)
                gap = across_span(rotation, line["bbox"])[0] - high
                previous_along = along_span(rotation, previous["bbox"])
                line_along = along_span(rotation, line["bbox"])
                overlap = min(previous_along[1], line_along[1]) - max(
                    previous_along[0], line_along[0]
                )
                if -height <= gap <= height * 1.4 and overlap > 0:
                    clusters[-1].append(line)
                    placed = True
            if not placed:
                clusters.append([line])
    return clusters


def _column_measure(lines: list[dict]) -> dict[tuple[int, int], float]:
    """How far the text runs, per left margin, across the whole page.

    A PDF records no margins, so a paragraph's measure has to be inferred. Its
    own widest line is the obvious guess and a poor one: a two-line paragraph
    whose lines both stop early would be re-wrapped into a column narrower than
    the one it sits in. What the other paragraphs starting at the same margin
    reach is a much better answer. The ninth decile rather than the maximum, so
    one line that overshoots cannot stretch the column for everything.
    """
    grouped: dict[tuple[int, int], list[float]] = {}
    for line in lines:
        rotation = line["rotation"]
        start, end = along_span(rotation, line["bbox"])
        grouped.setdefault((rotation, round(start)), []).append(end)

    measures: dict[tuple[int, int], float] = {}
    for key, ends in grouped.items():
        ends.sort()
        measures[key] = ends[min(int(len(ends) * 0.9), len(ends) - 1)]
    return measures


_CONTROL = re.compile(r"[\x00-\x08\x0b-\x1f\x7f]")


def clean(text: str) -> str:
    """Strip control characters a PDF's text can carry.

    Recognised text in particular is littered with them; written back into the
    page they would be drawn as empty boxes.
    """
    return _CONTROL.sub("", text)


def _is_recognised_span(span: dict) -> bool:
    """Whether a run came from reading a picture rather than from the file."""
    return span.get("alpha", 255) == 0 and "glyphless" in (span.get("font") or "").lower()


def _merge_spans(spans: list[dict]) -> list[dict]:
    """Join neighbouring runs that are set the same way.

    Recognition produces one run per word, which would leave the user editing a
    line a word at a time. Ordinary documents split runs too, wherever the
    writer happened to end one. Where the formatting matches there is nothing
    to keep apart.

    Sizes are compared exactly for text the file states, and loosely for text
    that was read off a picture: recognition estimates a size per word and they
    come out several per cent apart along a line that is plainly all one size.
    """
    merged: list[dict] = []
    sizes: list[list[float]] = []

    for span in spans:
        previous = merged[-1] if merged else None
        recognised = _is_recognised_span(span)
        tolerance = span["size"] * 0.15 if recognised else 0.01
        same = previous is not None and all(
            previous[key] == span[key] for key in ("font", "color", "flags", "alpha")
        )
        same = same and abs(previous["size"] - span["size"]) <= tolerance
        adjacent = same and abs(span["bbox"][0] - previous["bbox"][2]) <= max(
            1.0, span["size"] * 0.4
        )
        if same and adjacent:
            previous["text"] += span["text"]
            previous["bbox"] = [
                min(previous["bbox"][0], span["bbox"][0]),
                min(previous["bbox"][1], span["bbox"][1]),
                max(previous["bbox"][2], span["bbox"][2]),
                max(previous["bbox"][3], span["bbox"][3]),
            ]
            sizes[-1].append(span["size"])
        else:
            merged.append(span)
            sizes.append([span["size"]])

    # A merged run of estimated sizes takes their average, which is closer to
    # the size the page was really set in than any one word's guess.
    for span, values in zip(merged, sizes):
        if len(values) > 1:
            span["size"] = round(sum(values) / len(values), 2)
    return merged


def _dominant_style(line: dict) -> tuple[str, float]:
    """The typeface a line is mostly set in, as (font name, size)."""
    widest = max(line["spans"], key=lambda span: len(span["text"]))
    return (widest["font"], round(widest["size"], 1))


def _paragraph_clusters(lines: list[dict]) -> list[list[dict]]:
    """Group lines that are the same paragraph of running text.

    Stricter than :func:`_cluster_lines`, which only needs to see enough to
    guess alignment. Re-wrapping rewrites every line it is given, so a heading
    swept in with the body below it would be reflowed into that body. A line
    continues a paragraph only when it is set the same way, follows at the
    paragraph's own line spacing, starts at the same margin, and — the telling
    one — the line before it ran to the right margin instead of stopping short,
    because a short line is where a paragraph ends.
    """
    clusters: list[list[dict]] = []
    for rotation, group in _by_rotation(lines):
        for line in group:
            if clusters and clusters[-1][-1]["rotation"] == rotation and _continues(clusters[-1], line):
                clusters[-1].append(line)
            else:
                clusters.append([line])
    return clusters


def _by_rotation(lines: list[dict]) -> list[tuple[int, list[dict]]]:
    """Lines grouped by direction, each group in its own reading order.

    Sorting by plain page coordinates reads upside-down text backwards, which
    puts a paragraph's lines in reverse and makes each one look like the start
    of a new one.
    """
    grouped: dict[int, list[dict]] = {}
    for line in lines:
        grouped.setdefault(line["rotation"], []).append(line)
    for rotation, group in grouped.items():
        group.sort(
            key=lambda item: (
                across_span(rotation, item["bbox"])[0],
                along_span(rotation, item["bbox"])[0],
            )
        )
    return sorted(grouped.items())


def _continues(cluster: list[dict], line: dict) -> bool:
    """Whether ``line`` is the next line of ``cluster``'s paragraph."""
    previous = cluster[-1]
    rotation = line["rotation"]
    if previous["rotation"] != rotation:
        return False
    if _dominant_style(previous) != _dominant_style(line):
        return False

    previous_across = across_span(rotation, previous["bbox"])
    height = max(previous_across[1] - previous_across[0], 1.0)
    leading = across(rotation, *line["origin"]) - across(rotation, *previous["origin"])
    if not 0.7 * height <= leading <= 2.2 * height:
        return False

    # The paragraph's own starting margin: the first line may be indented, the
    # rest are not, so the margin is set by the second line onwards.
    reference = cluster[1] if len(cluster) > 1 else line
    margin = along_span(rotation, reference["bbox"])[0]
    if abs(along_span(rotation, line["bbox"])[0] - margin) > 2.0:
        return False

    # A line that stopped well short of the far margin ended its paragraph.
    far = max(along_span(rotation, item["bbox"])[1] for item in cluster)
    slack = 2.5 * _dominant_style(previous)[1]
    return along_span(rotation, previous["bbox"])[1] >= far - slack


def _detect_alignment(lines: list[dict]) -> str:
    """Guess how a paragraph's lines are aligned, so re-flow can preserve it.

    Only the lines' own edges are available, so this reads them the way a person
    would: a shared right edge with a ragged left one is right-aligned text.
    """
    if len(lines) < 2:
        return "left"
    rotation = lines[0]["rotation"]
    spans = [along_span(rotation, line["bbox"]) for line in lines]
    lefts = [span[0] for span in spans]
    rights = [span[1] for span in spans]
    centers = [(span[0] + span[1]) / 2 for span in spans]

    def spread(values: list[float]) -> float:
        return max(values) - min(values)

    left_even = spread(lefts) <= _ALIGN_TOLERANCE
    right_even = spread(rights) <= _ALIGN_TOLERANCE
    center_even = spread(centers) <= _ALIGN_TOLERANCE

    if left_even and right_even:
        return "left"  # justified, or every line the same width
    if right_even and not left_even:
        return "right"
    if center_even and not left_even:
        return "center"
    return "left"


def extract_page(
    doc: pymupdf.Document, pno: int, resolver: FontResolver | None = None
) -> dict[str, Any]:
    """Describe one page's editable text.

    Returns the page geometry plus its lines in reading order. Ids are positional
    (``p0-b3-l2``) and are only valid until the page is edited, which is why the
    client re-reads a page after every operation.
    """
    page = doc[pno]
    if resolver is not None:
        resolver.scan_page(pno)

    raw = page.get_text("dict")
    lines: list[dict[str, Any]] = []
    rotated = page.rotation != 0

    def rect(values) -> list[float]:
        box = pymupdf.Rect(values)
        if rotated:
            box = to_display(page, box)
        box.normalize()
        return [round(v, 2) for v in box]

    def point(values) -> list[float]:
        spot = pymupdf.Point(values)
        if rotated:
            spot = to_display(page, spot)
        return [round(spot.x, 2), round(spot.y, 2)]

    for bi, block in enumerate(raw.get("blocks", [])):
        if block.get("type") != 0:  # image blocks carry no editable text
            continue
        block_lines: list[dict[str, Any]] = []
        for li, line in enumerate(block.get("lines", [])):
            spans = []
            for si, span in enumerate(line.get("spans", [])):
                text = clean(span.get("text", ""))
                if not text:
                    continue
                font_name = span.get("font", "")
                flags = int(span.get("flags", 0))
                serif, mono, bold, italic = style_of(font_name, flags)
                faithful = True
                if resolver is not None:
                    faithful = resolver.resolve(pno, font_name, flags, text).is_faithful
                spans.append(
                    {
                        "id": f"p{pno}-b{bi}-l{li}-s{si}",
                        "text": text,
                        "font": font_name,
                        "size": round(float(span.get("size", 11.0)), 2),
                        "color": color_to_hex(int(span.get("color", 0))),
                        "flags": flags,
                        "bold": bold,
                        "italic": italic,
                        "mono": mono,
                        "serif": serif,
                        "bbox": rect(span.get("bbox", (0, 0, 0, 0))),
                        "origin": point(span.get("origin", (0, 0))),
                        "ascender": round(float(span.get("ascender", 0.8)), 4),
                        "descender": round(float(span.get("descender", -0.2)), 4),
                        "alpha": int(span.get("alpha", 255)),
                        "embedded": faithful,
                    }
                )
            if not spans:
                continue
            spans = _merge_spans(spans)
            for position, span in enumerate(spans):
                span["id"] = f"p{pno}-b{bi}-l{li}-s{position}"
            block_lines.append(
                {
                    "id": f"p{pno}-b{bi}-l{li}",
                    "page": pno,
                    "block": bi,
                    "bbox": rect(line.get("bbox", (0, 0, 0, 0))),
                    "origin": spans[0]["origin"],
                    "rotation": display_rotation(
                        page.rotation, rotation_from_dir(line.get("dir", (1.0, 0.0)))
                    ),
                    "dir": [round(v, 4) for v in line.get("dir", (1.0, 0.0))],
                    "spans": spans,
                    "text": "".join(span["text"] for span in spans),
                }
            )
        lines.extend(block_lines)

    # Alignment is read from loosely grouped neighbours, which is all it needs.
    for cluster in _cluster_lines(lines):
        alignment = _detect_alignment(cluster)
        for line in cluster:
            line["align"] = alignment

    # Paragraphs, for re-wrapping, are grouped far more strictly.
    columns = _column_measure(lines)
    for index, cluster in enumerate(_paragraph_clusters(lines)):
        box = [
            min(line["bbox"][0] for line in cluster),
            min(line["bbox"][1] for line in cluster),
            max(line["bbox"][2] for line in cluster),
            max(line["bbox"][3] for line in cluster),
        ]
        rotation = cluster[0]["rotation"]
        own_start, own_end = along_span(rotation, box)
        reach = max(own_end, columns.get((rotation, round(own_start)), own_end))
        # Reported back on the page, as the far corner the text may run to.
        measure_point = from_axes(rotation, reach, across(rotation, box[2], box[3]))
        for position, line in enumerate(cluster):
            line["block_bbox"] = [round(v, 2) for v in box]
            line["measure"] = round(reach, 2)
            line["measure_point"] = [round(v, 2) for v in measure_point]
            line["paragraph"] = f"p{pno}-par{index}"
            line["paragraph_index"] = position
            line["paragraph_size"] = len(cluster)

    return {
        "page": pno,
        "width": round(page.rect.width, 2),
        "height": round(page.rect.height, 2),
        "rotation": page.rotation,
        "lines": lines,
        "marks": page_marks(page),
    }


# Marks a reader can put on a page and take off again. A widget belongs to the
# document's form and a redaction is a pending instruction, so neither is one
# of these; a link has its own machinery.
MARK_TYPES = {"Highlight", "Underline", "StrikeOut", "Squiggly", "Text", "FreeText", "Square"}

# What a mark is drawn in when the file does not say.
DEFAULT_MARK_COLOR = "#ffd83d"


def page_marks(page: pymupdf.Page) -> list[dict[str, Any]]:
    """The annotations of a page, in the space the browser draws it in.

    Read out into plain values as they are visited: a live annotation belongs
    to the walk that produced it, and reading one after the fact is not safe.
    """
    marks = []
    for annot in page.annots():
        kind = annot.type[1]
        if kind not in MARK_TYPES:
            continue
        rect = to_display(page, pymupdf.Rect(annot.rect))
        stroke = annot.colors.get("stroke") or annot.colors.get("fill")
        marks.append(
            {
                "xref": annot.xref,
                "kind": kind.lower(),
                "bbox": [round(v, 2) for v in rect],
                "color": pdf_to_hex(stroke) or DEFAULT_MARK_COLOR,
                "note": annot.info.get("content", ""),
            }
        )
    return marks


def _inside_any(rect: pymupdf.Rect, boxes: list[pymupdf.Rect], slack: float = 1.0) -> bool:
    """Whether a box sits within one of `boxes`, give or take a point."""
    return any(
        box.x0 - slack <= rect.x0 and box.y0 - slack <= rect.y0
        and rect.x1 <= box.x1 + slack and rect.y1 <= box.y1 + slack
        for box in boxes
    )


def area_contents(page: pymupdf.Page, rect: pymupdf.Rect) -> dict[str, Any]:
    """What is inside a rectangle, so it can be said out loud before it goes.

    Erasing an area is not a covering-up: the glyphs come out of the file, and
    once the document is saved there is nothing left to recover them from.
    That is the right behaviour — a black box drawn over a name still has the
    name underneath it — but it is worth being sure about, and the only way to
    be sure is to be told what is about to go.

    What stays is worth saying too. A redaction takes page content; the marks
    and the form fields over it are not page content and survive, so a
    highlight over an erased sentence is left pointing at the gap.
    """
    area = pymupdf.Rect(rect)
    text = clean(page.get_text("text", clip=area) or "").strip()

    images = 0
    try:
        images = sum(
            1 for info in page.get_image_info() if area.intersects(pymupdf.Rect(info["bbox"]))
        )
    except Exception:  # pragma: no cover - defensive
        pass

    # An annotation paints itself, and its paint is reported among the page's
    # drawings with nothing to tell the two apart. Counting it would promise
    # to erase line art that is really a highlight — and that a redaction does
    # not touch anyway. What lies inside an annotation's box is its own.
    annot_boxes = [pymupdf.Rect(annot.rect) for annot in page.annots()]
    drawings = sum(
        1
        for drawing in page.get_drawings()
        if area.intersects(pymupdf.Rect(drawing["rect"]))
        and not _inside_any(pymupdf.Rect(drawing["rect"]), annot_boxes)
    )
    marks = sum(
        1 for annot in page.annots()
        if annot.type[1] in MARK_TYPES and area.intersects(pymupdf.Rect(annot.rect))
    )
    fields = sum(
        1 for widget in page.widgets() if area.intersects(pymupdf.Rect(widget.rect))
    )

    return {
        "text": text,
        "words": len(text.split()),
        "images": images,
        "drawings": drawings,
        "marks": marks,
        "fields": fields,
        "empty": not text and not images and not drawings,
    }


def page_summaries(
    doc: pymupdf.Document, only: Iterable[int] | None = None
) -> list[dict[str, Any]]:
    """Per-page geometry and state, for the rail and the client's page model.

    ``needs_ocr`` in particular is not cheap — it reads a page's pictures to
    say whether it is a scan — so ``only`` lets a caller ask for a handful of
    pages instead of paying that cost for every page on every call. Omitted,
    every page is summarised, in order, as before.
    """
    from .ocr import needs_ocr, was_recognised

    numbers = range(doc.page_count) if only is None else only
    summaries = []
    for pno in numbers:
        page = doc[pno]
        summaries.append(
            {
                "page": pno,
                "width": round(page.rect.width, 2),
                "height": round(page.rect.height, 2),
                "rotation": page.rotation,
                "needs_ocr": needs_ocr(page),
                "recognised": was_recognised(page),
            }
        )
    return summaries
