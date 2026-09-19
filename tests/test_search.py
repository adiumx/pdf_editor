"""Finding text across a document, and replacing it."""

from __future__ import annotations

import pymupdf
import pytest

from app.editor import apply_operations
from app.extract import extract_page
from app.fonts import FontResolver
from app.search import _fold, find, replace_operations
from tests.conftest import build_pdf, requires_fonts, text_of


def corpus() -> bytes:
    return build_pdf([
        ((72, 100), "Licenciado en Ciencias de la Computacion", "serif", 11),
        ((72, 120), "con mencion en Computación aplicada", "serif", 11),
        ((72, 150), "COMPUTACION en mayusculas", "serif-bold", 11),
        ((72, 180), "recomputacion no es palabra completa", "serif", 11),
    ])


@pytest.fixture
def doc_and_resolver():
    document = pymupdf.open(stream=corpus(), filetype="pdf")
    yield document, FontResolver(document)
    document.close()


class TestFolding:
    def test_accents_and_case_are_set_aside(self):
        assert _fold("Computación") == _fold("computacion")
        assert _fold("ÁÉÍÓÚñ") == "aeioun"


@requires_fonts
class TestFinding:
    def test_a_plain_search_ignores_case_and_accents(self, doc_and_resolver):
        """Someone typing "computacion" means to find "Computación"; asking for
        the accent hides the result behind a keystroke they cannot easily make."""
        document, resolver = doc_and_resolver
        assert len(find(document, resolver, "computacion")) == 4

    def test_matching_case_narrows_it(self, doc_and_resolver):
        document, resolver = doc_and_resolver
        found = find(document, resolver, "Computación", match_case=True)
        assert [match.text for match in found] == ["Computación"]

    def test_whole_word_excludes_the_middle_of_a_word(self, doc_and_resolver):
        document, resolver = doc_and_resolver
        found = find(document, resolver, "computacion", whole_word=True)
        assert all("recomputacion" not in match.line["text"] for match in found)
        assert len(found) == 3

    def test_nothing_is_found_for_an_empty_query(self, doc_and_resolver):
        document, resolver = doc_and_resolver
        assert find(document, resolver, "") == []

    def test_each_result_says_where_it_is(self, doc_and_resolver):
        document, resolver = doc_and_resolver
        match = find(document, resolver, "computacion")[0]
        entry = match.as_dict()
        assert entry["page"] == 0
        assert entry["rect"][2] > entry["rect"][0]
        assert entry["text"]
        assert entry["preview"]

    def test_the_rectangle_sits_over_the_text_it_found(self, doc_and_resolver):
        document, resolver = doc_and_resolver
        match = find(document, resolver, "Ciencias")[0]
        line = match.line["bbox"]
        assert line[0] - 1 <= match.rect[0] <= match.rect[2] <= line[2] + 1
        assert match.rect[1] >= line[1] - 1 and match.rect[3] <= line[3] + 1

    def test_offsets_survive_folding(self, doc_and_resolver):
        """Folding must not change the length, or a match would point at the
        wrong characters of the original text."""
        document, resolver = doc_and_resolver
        for match in find(document, resolver, "computacion"):
            assert _fold(match.text) == "computacion"


@requires_fonts
class TestReplacing:
    def _apply(self, document, resolver, *args, **kwargs):
        operations, count = replace_operations(document, resolver, *args, **kwargs)
        warnings = apply_operations(document, resolver, operations) if operations else []
        return count, warnings

    def test_every_occurrence_is_replaced(self, doc_and_resolver):
        document, resolver = doc_and_resolver
        count, _ = self._apply(document, resolver, "computacion", "INFORMATICA")
        assert count == 4
        text = text_of(document)
        assert text.count("INFORMATICA") == 4
        assert "Computación" not in text

    def test_the_rest_of_the_line_is_left_alone(self, doc_and_resolver):
        document, resolver = doc_and_resolver
        self._apply(document, resolver, "Computacion", "X")
        text = text_of(document)
        assert "Licenciado en Ciencias de la" in text
        assert "en mayusculas" in text

    def test_replacing_with_nothing_deletes_the_word(self, doc_and_resolver):
        document, resolver = doc_and_resolver
        count, _ = self._apply(document, resolver, "computacion", "", whole_word=True)
        assert count == 3
        assert "Computación" not in text_of(document)

    def test_a_query_that_matches_nothing_changes_nothing(self, doc_and_resolver):
        document, resolver = doc_and_resolver
        before = text_of(document)
        operations, count = replace_operations(document, resolver, "zzzz", "x")
        assert (operations, count) == ([], 0)
        assert text_of(document) == before

    def test_several_hits_on_one_line_are_all_replaced(self):
        document = pymupdf.open(
            stream=build_pdf([((72, 100), "uno dos uno tres uno", "serif", 11)]),
            filetype="pdf",
        )
        resolver = FontResolver(document)
        operations, count = replace_operations(document, resolver, "uno", "UNO")
        apply_operations(document, resolver, operations)
        assert count == 3
        assert text_of(document).count("UNO") == 3
        document.close()

    def test_a_longer_replacement_re_wraps_the_paragraph(self):
        """Replacing inside running text must not leave it in the margin."""
        rows = [
            ((72, 100 + 14 * i), " ".join(f"palabra{i}{n}" for n in range(9)), "serif", 11)
            for i in range(3)
        ]
        document = pymupdf.open(stream=build_pdf(rows), filetype="pdf")
        resolver = FontResolver(document)
        page = extract_page(document, 0, resolver)
        margin = max(line["measure"] for line in page["lines"])
        operations, count = replace_operations(
            document, resolver, "palabra00", "una palabra mucho mas larga que la anterior"
        )
        apply_operations(document, resolver, operations)
        after = extract_page(document, 0, FontResolver(document))
        assert count == 1
        for line in after["lines"]:
            assert line["bbox"][2] <= margin + 1, f"se sale: {line['text']!r}"
        document.close()


@requires_fonts
class TestSearchApi:
    @pytest.fixture
    def opened(self):
        from fastapi.testclient import TestClient

        from app.main import app, store

        with TestClient(app) as client:
            response = client.post(
                "/api/documents", files={"file": ("c.pdf", corpus(), "application/pdf")}
            )
            yield client, response.json()["id"]
        store.close_all()

    def test_search_returns_the_occurrences(self, opened):
        client, doc_id = opened
        body = client.post(f"/api/documents/{doc_id}/search", json={"query": "computacion"}).json()
        assert body["count"] == 4
        assert len(body["matches"]) == 4

    def test_replace_reports_how_many_it_changed(self, opened):
        client, doc_id = opened
        body = client.post(
            f"/api/documents/{doc_id}/replace",
            json={"query": "computacion", "replacement": "X"},
        ).json()
        assert body["replaced"] == 4
        assert body["can_undo"] is True

    def test_replace_is_one_undoable_step(self, opened):
        client, doc_id = opened
        client.post(
            f"/api/documents/{doc_id}/replace",
            json={"query": "computacion", "replacement": "X"},
        )
        client.post(f"/api/documents/{doc_id}/undo")
        page = client.get(f"/api/documents/{doc_id}/pages/0/text").json()
        assert any("Computación" in line["text"] for line in page["lines"])

    def test_an_empty_query_is_refused(self, opened):
        client, doc_id = opened
        assert client.post(f"/api/documents/{doc_id}/replace", json={"query": ""}).status_code == 400
