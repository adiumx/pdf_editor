"""Reading a page's text back out as editable pieces.

The editor works on *lines*: a line is the smallest run of text that can be
re-laid-out on its own, and it owns the spans (uniformly formatted runs) the
user actually types into. Every geometric fact the editor needs later to put
the text back — baseline origin, writing direction, alignment within its
paragraph — is captured here, because once a line is redacted it is gone.
"""

from __future__ import annotations

import math
from typing import Any, Iterable

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


def _cluster_lines(lines: list[dict]) -> list[list[dict]]:
    """Group lines that read as one paragraph.

    Alignment can only be seen across several lines, and PyMuPDF's own blocks
    are not a reliable paragraph: how a PDF's content stream happens to be
    written decides them, and plenty of files give every line a block of its
    own. Vertical adjacency plus horizontal overlap is what a reader actually
    uses, so that is what is used here.
    """
    clusters: list[list[dict]] = []
    for line in sorted(lines, key=lambda item: (item["bbox"][1], item["bbox"][0])):
        placed = False
        if clusters:
            previous = clusters[-1][-1]
            same_direction = previous["rotation"] == line["rotation"]
            height = max(previous["bbox"][3] - previous["bbox"][1], 1.0)
            gap = line["bbox"][1] - previous["bbox"][3]
            overlap = min(previous["bbox"][2], line["bbox"][2]) - max(
                previous["bbox"][0], line["bbox"][0]
            )
            if same_direction and -height <= gap <= height * 1.4 and overlap > 0:
                clusters[-1].append(line)
                placed = True
        if not placed:
            clusters.append([line])
    return clusters


def _detect_alignment(lines: list[dict]) -> str:
    """Guess how a paragraph's lines are aligned, so re-flow can preserve it.

    Only the lines' own edges are available, so this reads them the way a person
    would: a shared right edge with a ragged left one is right-aligned text.
    """
    if len(lines) < 2:
        return "left"
    lefts = [line["bbox"][0] for line in lines]
    rights = [line["bbox"][2] for line in lines]
    centers = [(line["bbox"][0] + line["bbox"][2]) / 2 for line in lines]

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
                text = span.get("text", "")
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

    # Alignment is decided across the whole page, not per block.
    for cluster in _cluster_lines(lines):
        alignment = _detect_alignment(cluster)
        paragraph = [
            min(line["bbox"][0] for line in cluster),
            min(line["bbox"][1] for line in cluster),
            max(line["bbox"][2] for line in cluster),
            max(line["bbox"][3] for line in cluster),
        ]
        for line in cluster:
            line["align"] = alignment
            line["block_bbox"] = [round(v, 2) for v in paragraph]

    return {
        "page": pno,
        "width": round(page.rect.width, 2),
        "height": round(page.rect.height, 2),
        "rotation": page.rotation,
        "lines": lines,
    }


def page_summaries(doc: pymupdf.Document) -> list[dict[str, Any]]:
    """Per-page geometry, for the thumbnail rail and the client's page model."""
    summaries = []
    for pno in range(doc.page_count):
        page = doc[pno]
        summaries.append(
            {
                "page": pno,
                "width": round(page.rect.width, 2),
                "height": round(page.rect.height, 2),
                "rotation": page.rotation,
            }
        )
    return summaries
