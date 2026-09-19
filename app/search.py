"""Finding text across a document, and replacing it.

A match is located inside one span — one run of text set a single way — because
that is the unit an edit can be expressed in. Text broken across a line ending
is therefore not matched: the words are there, but the file never joined them,
and guessing where the join belongs would find matches the user cannot see.
"""

from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass, field
from typing import Any, Iterator

import pymupdf

from .extract import along, along_span, across_span, extract_page, from_axes
from .fonts import FontResolver

# How much of the surrounding line to show beside a result.
_PREVIEW = 40


def _fold(text: str) -> str:
    """Strip accents and case, for matching the way a person searching would.

    Someone typing "computacion" means to find "Computación"; requiring the
    accent would hide the result behind a keystroke they cannot easily produce.
    """
    decomposed = unicodedata.normalize("NFD", text.lower())
    return "".join(ch for ch in decomposed if not unicodedata.combining(ch))


@dataclass
class Match:
    """One occurrence, and everything needed to show or replace it."""

    page: int
    line: dict[str, Any]
    span: int
    start: int
    end: int
    rect: list[float]
    preview: str

    @property
    def text(self) -> str:
        return self.line["spans"][self.span]["text"][self.start : self.end]

    def as_dict(self) -> dict[str, Any]:
        return {
            "page": self.page,
            "line": self.line["id"],
            "paragraph": self.line["paragraph"],
            "span": self.span,
            "start": self.start,
            "end": self.end,
            "rect": self.rect,
            "text": self.text,
            "preview": self.preview,
        }


def _pattern(query: str, *, match_case: bool, whole_word: bool) -> re.Pattern[str]:
    """The expression to scan a span with, built from the search options."""
    body = re.escape(query if match_case else _fold(query))
    if whole_word:
        body = rf"(?<!\w){body}(?!\w)"
    return re.compile(body)


def _searchable(text: str, match_case: bool) -> str:
    """The span's text as the pattern expects it, character for character.

    Folding must not change the length, or the offsets a match reports would no
    longer point at the original text. Decomposing and dropping the combining
    marks can shorten it, so each character is folded on its own and anything
    that does not survive is kept as it was.
    """
    if match_case:
        return text
    folded = []
    for char in text:
        reduced = _fold(char)
        folded.append(reduced if len(reduced) == 1 else char.lower())
    return "".join(folded)


def _match_rect(
    resolver: FontResolver, page: int, line: dict, span_index: int, start: int, end: int
) -> list[float]:
    """Where an occurrence sits on the page, for highlighting it."""
    span = line["spans"][span_index]
    rotation = line["rotation"]
    font = resolver.resolve(page, span["font"], span.get("flags", 0), span["text"])
    size = float(span["size"])
    begin = along_span(rotation, span["bbox"])[0] + font.text_length(span["text"][:start], size)
    finish = begin + font.text_length(span["text"][start:end], size)
    low, high = across_span(rotation, span["bbox"])
    corners = [from_axes(rotation, begin, low), from_axes(rotation, finish, high)]
    return [
        round(min(corners[0][0], corners[1][0]), 2),
        round(min(corners[0][1], corners[1][1]), 2),
        round(max(corners[0][0], corners[1][0]), 2),
        round(max(corners[0][1], corners[1][1]), 2),
    ]


def find(
    doc: pymupdf.Document,
    resolver: FontResolver,
    query: str,
    *,
    match_case: bool = False,
    whole_word: bool = False,
    pages: list[dict[str, Any]] | None = None,
) -> list[Match]:
    """Every occurrence of ``query``, in reading order."""
    if not query:
        return []
    pattern = _pattern(query, match_case=match_case, whole_word=whole_word)
    results: list[Match] = []

    for pno in range(doc.page_count):
        page = pages[pno] if pages else extract_page(doc, pno, resolver)
        for line in page["lines"]:
            for index, span in enumerate(line["spans"]):
                text = span["text"]
                for found in pattern.finditer(_searchable(text, match_case)):
                    start, end = found.span()
                    if start == end:
                        continue
                    results.append(
                        Match(
                            page=pno,
                            line=line,
                            span=index,
                            start=start,
                            end=end,
                            rect=_match_rect(resolver, pno, line, index, start, end),
                            preview=_preview_of(line["text"], span["text"], start, end),
                        )
                    )
    return results


def _preview_of(line_text: str, span_text: str, start: int, end: int) -> str:
    """A snippet of the line around the occurrence, to show in the results."""
    offset = line_text.find(span_text)
    if offset < 0:
        offset = 0
    at = offset + start
    to = at + (end - start)
    left = max(0, at - _PREVIEW // 2)
    right = min(len(line_text), to + _PREVIEW // 2)
    snippet = line_text[left:right].strip()
    return ("…" if left > 0 else "") + snippet + ("…" if right < len(line_text) else "")


def replace_operations(
    doc: pymupdf.Document,
    resolver: FontResolver,
    query: str,
    replacement: str,
    *,
    match_case: bool = False,
    whole_word: bool = False,
) -> tuple[list[dict[str, Any]], int]:
    """The edits that carry out a replace-all, and how many it makes.

    One operation per paragraph, with every occurrence in it already applied, so
    a paragraph is re-broken once however many times the word appears in it.
    """
    pages = [extract_page(doc, pno, resolver) for pno in range(doc.page_count)]
    matches = find(
        doc, resolver, query, match_case=match_case, whole_word=whole_word, pages=pages
    )
    if not matches:
        return [], 0

    # Group by paragraph, and within it note which lines and spans changed.
    grouped: dict[tuple[int, str], list[Match]] = {}
    for match in matches:
        grouped.setdefault((match.page, match.line["paragraph"]), []).append(match)

    operations: list[dict[str, Any]] = []
    count = 0

    for (pno, paragraph), group in grouped.items():
        lines = [line for line in pages[pno]["lines"] if line["paragraph"] == paragraph]
        index_of = {line["id"]: position for position, line in enumerate(lines)}
        payload = [
            {**line, "spans": [dict(span) for span in line["spans"]]} for line in lines
        ]

        touched_lines: set[int] = set()
        first: tuple[int, int] | None = None

        # Right to left, so an earlier replacement cannot shift a later offset.
        for match in sorted(
            group, key=lambda m: (index_of[m.line["id"]], m.span, m.start), reverse=True
        ):
            position = index_of[match.line["id"]]
            span = payload[position]["spans"][match.span]
            span["text"] = span["text"][: match.start] + replacement + span["text"][match.end :]
            touched_lines.add(position)
            first = (position, match.span)
            count += 1

        operations.append(
            {
                "op": "replace_paragraph",
                "page": pno,
                "box": lines[0]["block_bbox"],
                "measure_point": lines[0].get("measure_point"),
                "align": lines[0]["align"],
                "lines": payload,
                "edited": {"line": first[0], "span": first[1]},
                # With more than one line changed, only a full re-break applies
                # them all: the cheap single-line path would drop the rest.
                "reflow": len(touched_lines) > 1,
            }
        )

    return operations, count
