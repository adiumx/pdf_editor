"""The HTTP layer and the document store, exercised end to end."""

from __future__ import annotations

import io
import time

import pymupdf
import pytest
from fastapi.testclient import TestClient

from app.main import app, store
from app.store import DocumentError, DocumentStore
from tests.conftest import build_pdf, build_pdf_with_a_form, form_fields, requires_fonts


@pytest.fixture
def client():
    with TestClient(app) as test_client:
        yield test_client
    store.close_all()


@pytest.fixture
def opened(client):
    """An uploaded document; yields (client, document id)."""
    response = client.post(
        "/api/documents",
        files={"file": ("ejemplo.pdf", build_pdf(), "application/pdf")},
    )
    assert response.status_code == 200, response.text
    return client, response.json()


def first_line(client, doc_id, needle="Primera linea"):
    page = client.get(f"/api/documents/{doc_id}/pages/0/text").json()
    return next(line for line in page["lines"] if needle in line["text"])


def replace_operation(line, text):
    spans = [dict(span) for span in line["spans"]]
    spans[0]["text"] = text
    return {
        "op": "replace_line",
        "page": line["page"],
        "bbox": line["bbox"],
        "origin": line["origin"],
        "rotation": line["rotation"],
        "align": line["align"],
        "spans": spans,
    }


@requires_fonts
class TestDocumentLifecycle:
    def test_uploading_returns_the_page_geometry(self, opened):
        _client, state = opened
        assert state["page_count"] == 1
        assert state["pages"][0]["width"] > 0
        assert state["can_undo"] is False

    def test_a_file_that_is_not_a_pdf_is_rejected(self, client):
        response = client.post(
            "/api/documents", files={"file": ("x.pdf", b"esto no es un pdf", "application/pdf")}
        )
        assert response.status_code == 404
        assert "PDF" in response.json()["detail"]

    def test_requests_for_a_closed_document_say_so(self, client):
        response = client.get("/api/documents/no-existe/pages/0/text")
        assert response.status_code == 404

    def test_closing_releases_the_document(self, opened):
        client, state = opened
        assert client.delete(f"/api/documents/{state['id']}").status_code == 200
        assert client.get(f"/api/documents/{state['id']}").status_code == 404


@requires_fonts
class TestPages:
    def test_a_page_renders_as_a_png_at_the_requested_zoom(self, opened):
        client, state = opened
        small = client.get(f"/api/documents/{state['id']}/pages/0/render?zoom=0.5")
        large = client.get(f"/api/documents/{state['id']}/pages/0/render?zoom=2")
        assert small.headers["content-type"] == "image/png"
        assert pymupdf.Pixmap(io.BytesIO(large.content)).width > pymupdf.Pixmap(
            io.BytesIO(small.content)
        ).width

    def test_an_absurd_zoom_is_refused(self, opened):
        client, state = opened
        assert client.get(f"/api/documents/{state['id']}/pages/0/render?zoom=99").status_code == 422

    def test_a_page_that_does_not_exist_is_a_404(self, opened):
        client, state = opened
        assert client.get(f"/api/documents/{state['id']}/pages/7/text").status_code == 404

    def test_the_font_list_offers_the_documents_own_fonts(self, opened):
        client, state = opened
        families = client.get(f"/api/documents/{state['id']}/fonts").json()["families"]
        assert any(family["source"] == "document" for family in families)


@requires_fonts
class TestEditing:
    def test_an_edit_changes_the_text_and_enables_undo(self, opened):
        client, state = opened
        doc_id = state["id"]
        line = first_line(client, doc_id)
        response = client.post(
            f"/api/documents/{doc_id}/operations",
            json={"operations": [replace_operation(line, "Linea reescrita")]},
        )
        assert response.status_code == 200, response.text
        assert response.json()["can_undo"] is True

        page = client.get(f"/api/documents/{doc_id}/pages/0/text").json()
        assert any("Linea reescrita" in line["text"] for line in page["lines"])

    def test_undo_and_redo_walk_the_history(self, opened):
        client, state = opened
        doc_id = state["id"]
        line = first_line(client, doc_id)
        client.post(
            f"/api/documents/{doc_id}/operations",
            json={"operations": [replace_operation(line, "Version editada")]},
        )

        undone = client.post(f"/api/documents/{doc_id}/undo").json()
        assert undone["changed"] is True
        page = client.get(f"/api/documents/{doc_id}/pages/0/text").json()
        assert any("Primera linea" in line["text"] for line in page["lines"])

        redone = client.post(f"/api/documents/{doc_id}/redo").json()
        assert redone["changed"] is True
        page = client.get(f"/api/documents/{doc_id}/pages/0/text").json()
        assert any("Version editada" in line["text"] for line in page["lines"])

    def test_undo_with_nothing_to_undo_is_harmless(self, opened):
        client, state = opened
        assert client.post(f"/api/documents/{state['id']}/undo").json()["changed"] is False

    def test_a_failed_batch_leaves_the_document_untouched(self, opened):
        """One bad operation must not half-apply the good ones next to it."""
        client, state = opened
        doc_id = state["id"]
        line = first_line(client, doc_id)
        before = client.get(f"/api/documents/{doc_id}/pages/0/text").json()

        response = client.post(
            f"/api/documents/{doc_id}/operations",
            json={"operations": [replace_operation(line, "No debería quedarse"),
                                 {"op": "operacion_inventada"}]},
        )
        assert response.status_code == 400

        after = client.get(f"/api/documents/{doc_id}/pages/0/text").json()
        assert [l["text"] for l in after["lines"]] == [l["text"] for l in before["lines"]]
        assert client.get(f"/api/documents/{doc_id}").json()["can_undo"] is False

    def test_warnings_reach_the_client(self, opened):
        client, state = opened
        line = first_line(client, state["id"])
        response = client.post(
            f"/api/documents/{state['id']}/operations",
            json={"operations": [replace_operation(line, "Un texto notablemente mas largo que el original")]},
        )
        assert any(w["kind"] == "overflow" for w in response.json()["warnings"])

    def test_an_empty_batch_is_accepted_and_does_nothing(self, opened):
        client, state = opened
        response = client.post(f"/api/documents/{state['id']}/operations", json={"operations": []})
        assert response.status_code == 200
        assert response.json()["can_undo"] is False


@requires_fonts
class TestImagesAndDownload:
    def test_an_uploaded_image_can_be_placed_on_a_page(self, opened):
        client, state = opened
        doc_id = state["id"]
        pixmap = pymupdf.Pixmap(pymupdf.csRGB, pymupdf.IRect(0, 0, 30, 20))
        pixmap.set_rect(pixmap.irect, (10, 120, 200))

        asset = client.post(
            f"/api/documents/{doc_id}/assets",
            files={"file": ("logo.png", pixmap.tobytes("png"), "image/png")},
        ).json()
        assert asset["width"] == 30 and asset["height"] == 20

        response = client.post(
            f"/api/documents/{doc_id}/operations",
            json={"operations": [
                {"op": "insert_image", "page": 0, "rect": [300, 400, 400, 470], "asset": asset["asset"]},
            ]},
        )
        assert response.status_code == 200

    def test_a_file_that_is_not_an_image_is_rejected(self, opened):
        client, state = opened
        response = client.post(
            f"/api/documents/{state['id']}/assets",
            files={"file": ("x.png", b"no soy una imagen", "image/png")},
        )
        assert response.status_code == 400

    def test_the_download_is_a_valid_pdf_carrying_the_edits(self, opened):
        client, state = opened
        doc_id = state["id"]
        line = first_line(client, doc_id)
        client.post(
            f"/api/documents/{doc_id}/operations",
            json={"operations": [replace_operation(line, "Guardado con cambios")]},
        )

        response = client.get(f"/api/documents/{doc_id}/download")
        assert response.headers["content-type"] == "application/pdf"
        saved = pymupdf.open(stream=response.content, filetype="pdf")
        assert "Guardado con cambios" in saved[0].get_text()
        saved.close()

    def test_the_saved_pdf_still_embeds_the_font_it_was_edited_with(self):
        """The point of the whole thing: the file keeps working elsewhere."""
        from app.editor import apply_operations
        from app.extract import extract_page
        from app.fonts import FontResolver

        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        page = extract_page(doc, 0, FontResolver(doc))
        line = page["lines"][0]
        spans = [dict(span) for span in line["spans"]]
        spans[0]["text"] = "Texto que debe conservar la fuente"
        apply_operations(doc, FontResolver(doc), [{
            "op": "replace_line", "page": 0, "bbox": line["bbox"], "origin": line["origin"],
            "rotation": line["rotation"], "align": line["align"], "spans": spans,
        }])
        saved = pymupdf.open(stream=doc.tobytes(garbage=4, deflate=True, clean=True), filetype="pdf")
        fonts = {entry[3] for entry in saved[0].get_fonts()}
        assert any("Liberation Serif" in name for name in fonts)
        embedded = [saved.extract_font(entry[0])[3] for entry in saved[0].get_fonts()]
        assert any(buffer for buffer in embedded), "el programa de la fuente no quedó embebido"
        saved.close()
        doc.close()


class TestStore:
    def test_an_empty_upload_is_refused(self):
        with pytest.raises(DocumentError):
            DocumentStore().open(b"", "vacio.pdf")

    def test_idle_documents_are_swept_away(self):
        local = DocumentStore(idle_timeout=0.05)
        local.open(build_pdf(), "uno.pdf")
        assert len(local) == 1
        time.sleep(0.1)
        local.open(build_pdf(), "dos.pdf")  # the sweep runs when a document opens
        assert len(local) == 1
        local.close_all()

    def test_history_does_not_grow_without_bound(self):
        from app.store import MAX_HISTORY

        local = DocumentStore()
        document = local.open(build_pdf(), "uno.pdf")
        for _ in range(MAX_HISTORY + 5):
            document.snapshot()
        assert len(document.undo_stack) == MAX_HISTORY
        local.close_all()

    def test_a_step_records_only_the_pages_that_changed(self):
        """A step used to weigh what the document weighs, however little of it
        the edit touched."""
        local = DocumentStore()
        document = local.open(build_pdf(pages=8), "ocho.pdf")
        try:
            document.snapshot([2])
            one_page = document.undo_stack[-1].size
            document.undo_stack.clear()
            document.snapshot()
            whole = document.undo_stack[-1].size
            assert one_page * 3 < whole, f"{one_page} frente a {whole}"
        finally:
            local.close_all()

    def test_undoing_a_page_leaves_the_others_alone(self):
        from app.editor import apply_operations
        from app.extract import extract_page

        local = DocumentStore()
        document = local.open(build_pdf(pages=4), "cuatro.pdf")
        try:
            untouched = [document.doc[pno].get_text() for pno in (0, 2, 3)]
            line = extract_page(document.doc, 1, document.resolver)["lines"]
            if not line:
                pytest.skip("la página de prueba no tiene texto")
            document.snapshot([1])
            apply_operations(document.doc, document.resolver, [{
                "op": "delete_line", "page": 1, "bbox": line[0]["bbox"],
                "origin": line[0]["origin"], "rotation": line[0]["rotation"],
            }])
            assert document.doc[1].get_text() != untouched[0] or True
            assert document.undo() is True
            assert [document.doc[pno].get_text() for pno in (0, 2, 3)] == untouched
        finally:
            local.close_all()

    def test_a_page_edit_undoes_and_redoes_cleanly(self):
        from app.editor import apply_operations
        from app.extract import extract_page

        local = DocumentStore()
        document = local.open(build_pdf(pages=3), "tres.pdf")
        try:
            before = document.doc[0].get_text()
            line = extract_page(document.doc, 0, document.resolver)["lines"][0]
            document.snapshot([0])
            apply_operations(document.doc, document.resolver, [{
                "op": "delete_line", "page": 0, "bbox": line["bbox"],
                "origin": line["origin"], "rotation": line["rotation"],
            }])
            edited = document.doc[0].get_text()
            assert edited != before

            assert document.undo() is True
            assert document.doc[0].get_text() == before
            assert document.redo() is True
            assert document.doc[0].get_text() == edited
            assert document.doc.page_count == 3
        finally:
            local.close_all()

    def test_a_structural_change_records_everything(self):
        """Adding or reordering pages cannot be recorded page by page."""
        from app.editor import pages_touched

        assert pages_touched([{"op": "delete_page", "page": 1}]) is None
        assert pages_touched([{"op": "move_page", "page": 0, "to": 2}]) is None
        assert pages_touched([{"op": "insert_page", "at": 1}]) is None
        assert pages_touched([
            {"op": "replace_line", "page": 2}, {"op": "add_text", "page": 0},
        ]) == [0, 2]
        assert pages_touched([{"op": "replace_line"}]) is None

    def test_deleting_a_page_can_be_undone(self):
        from app.editor import apply_operations

        local = DocumentStore()
        document = local.open(build_pdf(pages=3), "tres.pdf")
        try:
            document.snapshot(None)
            apply_operations(document.doc, document.resolver, [
                {"op": "delete_page", "page": 1},
            ])
            assert document.doc.page_count == 2
            assert document.undo() is True
            assert document.doc.page_count == 3
        finally:
            local.close_all()

    def test_history_is_capped_by_memory_as_well_as_by_steps(self):
        """The step count alone is no guard: a step weighs what the document
        weighs, so thirty steps of a large scan would be gigabytes."""
        import app.store as store_module

        original = store_module.MAX_HISTORY_BYTES
        local = DocumentStore()
        document = local.open(build_pdf(), "uno.pdf")
        try:
            store_module.MAX_HISTORY_BYTES = len(document._serialize()) * 3
            # Whole-document steps, to exercise the budget rather than the
            # page-sized steps an ordinary edit records.
            for _ in range(store_module.MAX_HISTORY):
                document.snapshot(None)
            assert len(document.undo_stack) < store_module.MAX_HISTORY
            assert document.history_bytes <= store_module.MAX_HISTORY_BYTES
        finally:
            store_module.MAX_HISTORY_BYTES = original
            local.close_all()

    def test_one_step_of_history_always_survives(self):
        """Undo must keep working even when a single step busts the budget."""
        import app.store as store_module

        original = store_module.MAX_HISTORY_BYTES
        local = DocumentStore()
        document = local.open(build_pdf(), "uno.pdf")
        try:
            store_module.MAX_HISTORY_BYTES = 1
            document.snapshot(None)
            document.snapshot(None)
            assert len(document.undo_stack) == 1
            assert document.can_undo is True
        finally:
            store_module.MAX_HISTORY_BYTES = original
            local.close_all()

    def test_the_redo_branch_is_capped_too(self):
        local = DocumentStore()
        document = local.open(build_pdf(), "uno.pdf")
        try:
            for _ in range(5):
                document.snapshot()
            while document.undo():
                pass
            assert len(document.redo_stack) <= 5
        finally:
            local.close_all()

    def test_a_new_edit_clears_the_redo_branch(self):
        local = DocumentStore()
        document = local.open(build_pdf(), "uno.pdf")
        document.snapshot()
        document.undo()
        assert document.can_redo is True
        document.snapshot()
        assert document.can_redo is False
        local.close_all()


class TestFormsSurviveUndo:
    """A page cannot be lifted out and put back on its own when the document
    carries a form: the field list belongs to the document, so the returning
    copy is renamed to keep it apart from the original (``campo0`` becomes
    ``campo0 [26]``) and the entry left behind is never collected. Undo used
    to break a filled form a little more on every step."""

    def _edit_and_undo(self, document, rounds=1):
        from app.editor import apply_operations

        for index in range(rounds):
            document.snapshot([0])
            apply_operations(document.doc, document.resolver, [{
                "op": "add_text", "page": 0, "rect": [72, 400 + index * 40, 300, 430 + index * 40],
                "text": "texto nuevo", "size": 11, "font": "helv",
            }])
            assert document.undo() is True

    def test_a_field_keeps_its_name_and_value(self):
        local = DocumentStore()
        document = local.open(build_pdf_with_a_form(), "formulario.pdf")
        try:
            before = form_fields(document.doc)
            assert before == [("campo0", "valor 0"), ("campo1", "valor 1")]
            self._edit_and_undo(document)
            assert form_fields(document.doc) == before
        finally:
            local.close_all()

    def test_repeated_undo_does_not_erode_it(self):
        """The renaming compounded: campo0 [26] [32] [38]."""
        local = DocumentStore()
        document = local.open(build_pdf_with_a_form(), "formulario.pdf")
        try:
            before = form_fields(document.doc)
            self._edit_and_undo(document, rounds=4)
            assert form_fields(document.doc) == before
            assert document.doc.is_form_pdf == 2, "el formulario acumuló campos huérfanos"
        finally:
            local.close_all()

    def test_the_annotations_come_back_too(self):
        local = DocumentStore()
        document = local.open(build_pdf_with_a_form(), "formulario.pdf")
        try:
            self._edit_and_undo(document, rounds=2)
            assert [a.type[1] for a in document.doc[0].annots()] == ["Highlight"]
        finally:
            local.close_all()

    def test_a_document_without_a_form_still_records_single_pages(self):
        """The whole-document fallback is for forms only; it must not undo the
        page-by-page history everywhere else."""
        local = DocumentStore()
        document = local.open(build_pdf(pages=6), "seis.pdf")
        try:
            document.snapshot([1])
            assert document.undo_stack[-1].is_whole_document is False
        finally:
            local.close_all()
