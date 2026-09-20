"""Reading and filling a PDF's interactive form."""

from __future__ import annotations

import pymupdf
import pytest

from app.editor import EditError, apply_operations
from app.fonts import FontResolver
from app.forms import FormError, page_fields, set_field, signed_field_names
from app.store import DocumentStore
from tests.conftest import (
    build_pdf,
    build_pdf_with_every_kind_of_field,
    field_named,
    sign_field,
)


@pytest.fixture
def form():
    doc = pymupdf.open(stream=build_pdf_with_every_kind_of_field(), filetype="pdf")
    yield doc
    doc.close()


class TestReadingTheForm:
    def test_every_field_is_found(self, form):
        names = [f["name"] for f in page_fields(form[0])]
        assert names.count("respuesta") == 3, "faltan botones del grupo"
        assert set(names) == {"nombre", "corto", "fijo", "acepto", "pais", "plan",
                              "firma", "respuesta"}

    def test_each_kind_is_named_as_the_browser_expects(self, form):
        kinds = {f["name"]: f["kind"] for f in page_fields(form[0])}
        assert kinds == {
            "nombre": "text", "corto": "text", "fijo": "text",
            "acepto": "checkbox", "pais": "combo", "plan": "list",
            "firma": "signature", "respuesta": "radio",
        }

    def test_a_tick_reads_as_true_or_false_not_as_its_state_name(self, form):
        """The on state is called something different in every document."""
        assert field_named(form, "acepto")["value"] is False
        set_field(form[0], field_named(form, "acepto")["xref"], True)
        assert field_named(form, "acepto")["value"] is True

    def test_the_choices_come_with_the_field(self, form):
        assert field_named(form, "pais")["options"] == ["España", "Francia", "Portugal"]

    def test_a_locked_field_says_so(self, form):
        assert field_named(form, "fijo")["readonly"] is True
        assert field_named(form, "nombre")["readonly"] is False

    def test_a_signature_counts_as_unfillable(self, form):
        """Nothing here has a certificate, so it is not offered as editable."""
        assert field_named(form, "firma")["readonly"] is True

    def test_the_length_limit_is_reported(self, form):
        assert field_named(form, "corto")["maxlen"] == 4

    def test_a_document_without_a_form_has_no_fields(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        assert page_fields(doc[0]) == []
        doc.close()


class TestFillingItIn:
    def test_typing_into_a_text_field(self, form):
        xref = field_named(form, "nombre")["xref"]
        assert set_field(form[0], xref, "Beatriz Lopez")["value"] == "Beatriz Lopez"
        assert field_named(form, "nombre")["value"] == "Beatriz Lopez"

    def test_choosing_from_a_list(self, form):
        xref = field_named(form, "plan")["xref"]
        set_field(form[0], xref, "Pro")
        assert field_named(form, "plan")["value"] == "Pro"

    def test_a_value_the_list_does_not_offer_is_refused(self, form):
        with pytest.raises(FormError):
            set_field(form[0], field_named(form, "plan")["xref"], "Platino")

    def test_a_combo_takes_what_is_typed_into_it(self, form):
        """A combo box may allow free text; a list box may not."""
        xref = field_named(form, "pais")["xref"]
        set_field(form[0], xref, "Andorra")
        assert field_named(form, "pais")["value"] == "Andorra"

    def test_too_long_a_value_is_refused_rather_than_cut(self, form):
        with pytest.raises(FormError) as raised:
            set_field(form[0], field_named(form, "corto")["xref"], "demasiado largo")
        assert "4" in str(raised.value)
        assert field_named(form, "corto")["value"] == "ab", "se escribió igualmente"

    def test_a_locked_field_is_refused(self, form):
        with pytest.raises(FormError):
            set_field(form[0], field_named(form, "fijo")["xref"], "otra cosa")
        assert field_named(form, "fijo")["value"] == "no tocar"

    def test_a_signature_is_refused_with_a_reason(self, form):
        with pytest.raises(FormError) as raised:
            set_field(form[0], field_named(form, "firma")["xref"], "yo")
        assert "firma" in str(raised.value).lower()

    def test_a_field_that_is_gone_is_refused(self, form):
        with pytest.raises(FormError):
            set_field(form[0], 999999, "x")

    def test_ticking_one_button_unticks_its_siblings(self, form):
        """They share one value, held by the field they all hang off."""
        third = field_named(form, "respuesta", which=2)["xref"]
        set_field(form[0], third, True)
        values = [f["value"] for f in page_fields(form[0]) if f["name"] == "respuesta"]
        assert values == [False, False, True]

        first = field_named(form, "respuesta", which=0)["xref"]
        set_field(form[0], first, True)
        values = [f["value"] for f in page_fields(form[0]) if f["name"] == "respuesta"]
        assert values == [True, False, False], "quedaron dos marcados"

    def test_clearing_a_text_field(self, form):
        xref = field_named(form, "nombre")["xref"]
        set_field(form[0], xref, "")
        assert field_named(form, "nombre")["value"] == ""


class TestThroughTheOperation:
    def _doc(self):
        return pymupdf.open(stream=build_pdf_with_every_kind_of_field(), filetype="pdf")

    def test_the_operation_fills_the_field(self):
        doc = self._doc()
        resolver = FontResolver(doc)
        apply_operations(doc, resolver, [{
            "op": "fill_field", "page": 0,
            "xref": field_named(doc, "nombre")["xref"], "value": "Cesar",
        }])
        assert field_named(doc, "nombre")["value"] == "Cesar"
        doc.close()

    def test_a_refusal_comes_back_as_an_edit_error(self):
        doc = self._doc()
        resolver = FontResolver(doc)
        with pytest.raises(EditError):
            apply_operations(doc, resolver, [{
                "op": "fill_field", "page": 0,
                "xref": field_named(doc, "fijo")["xref"], "value": "x",
            }])
        doc.close()

    def test_filling_a_field_is_undoable(self):
        local = DocumentStore()
        document = local.open(build_pdf_with_every_kind_of_field(), "formulario.pdf")
        try:
            xref = field_named(document.doc, "nombre")["xref"]
            document.snapshot([0])
            apply_operations(document.doc, document.resolver, [{
                "op": "fill_field", "page": 0, "xref": xref, "value": "Cesar",
            }])
            assert field_named(document.doc, "nombre")["value"] == "Cesar"
            assert document.undo() is True
            assert field_named(document.doc, "nombre")["value"] == "Ana"
        finally:
            local.close_all()

    def test_editing_the_page_leaves_the_form_alone(self):
        """A field is not page content, so a redaction must not reach it."""
        doc = self._doc()
        resolver = FontResolver(doc)
        before = [(f["name"], f["value"]) for f in page_fields(doc[0])]
        apply_operations(doc, resolver, [{
            "op": "add_text", "page": 0, "rect": [72, 600, 300, 630],
            "text": "texto nuevo", "size": 11, "font": "helv",
        }])
        assert [(f["name"], f["value"]) for f in page_fields(doc[0])] == before
        doc.close()

    def test_the_value_survives_saving_and_reopening(self):
        doc = self._doc()
        resolver = FontResolver(doc)
        apply_operations(doc, resolver, [{
            "op": "fill_field", "page": 0,
            "xref": field_named(doc, "nombre")["xref"], "value": "Cesar Ruiz",
        }])
        again = pymupdf.open(stream=doc.tobytes(), filetype="pdf")
        assert field_named(again, "nombre")["value"] == "Cesar Ruiz"
        again.close()
        doc.close()


class TestHowTheHistoryTreatsAForm:
    """A page showing a form field cannot be recorded on its own, so a step
    covering one records the whole document instead."""

    def _mixed(self, pages=4):
        """A document whose first page carries the form and whose rest do not."""
        form = pymupdf.open(stream=build_pdf_with_every_kind_of_field(), filetype="pdf")
        plain = pymupdf.open(stream=build_pdf(pages=pages - 1), filetype="pdf")
        form.insert_pdf(plain, annots=True, links=True)
        data = form.tobytes()
        plain.close()
        form.close()
        return data

    def test_a_page_with_fields_records_the_whole_document(self):
        local = DocumentStore()
        document = local.open(self._mixed(), "mixto.pdf")
        try:
            document.snapshot([0])
            assert document.undo_stack[-1].is_whole_document is True
        finally:
            local.close_all()

    def test_a_page_without_fields_still_records_only_itself(self):
        """In a long form, the pages with no fields keep their cheap steps."""
        local = DocumentStore()
        document = local.open(self._mixed(), "mixto.pdf")
        try:
            document.snapshot([2])
            assert document.undo_stack[-1].is_whole_document is False
        finally:
            local.close_all()

    def test_the_form_survives_repeated_undo(self):
        """The renaming used to compound: campo0 [26] [32] [38]."""
        local = DocumentStore()
        document = local.open(build_pdf_with_every_kind_of_field(), "formulario.pdf")
        try:
            before = [(f["name"], f["value"]) for f in page_fields(document.doc[0])]
            for round_number in range(4):
                document.snapshot([0])
                apply_operations(document.doc, document.resolver, [{
                    "op": "add_text", "page": 0,
                    "rect": [320, 100 + round_number * 40, 560, 130 + round_number * 40],
                    "text": "texto", "size": 11, "font": "helv",
                }])
                assert document.undo() is True
            assert [(f["name"], f["value"]) for f in page_fields(document.doc[0])] == before
        finally:
            local.close_all()

    def test_a_form_whose_field_list_is_malformed_is_still_protected(self):
        """``is_form_pdf`` reads the document's field list, and a document
        whose list is broken answers no while its pages carry fields all the
        same. The pages are asked instead."""
        doc = pymupdf.open(stream=build_pdf_with_every_kind_of_field(), filetype="pdf")
        doc.xref_set_key(-1, "Root/AcroForm/Fields", "[]")
        data = doc.tobytes()
        doc.close()

        local = DocumentStore()
        document = local.open(data, "roto.pdf")
        try:
            assert not document.doc.is_form_pdf, "el banco no quedó malformado"
            assert document.doc[0].first_widget is not None, "la página perdió los campos"
            document.snapshot([0])
            assert document.undo_stack[-1].is_whole_document is True
        finally:
            local.close_all()


class TestWarningAboutASignature:
    """A signature field is only worth warning about once it actually holds a
    signature — a place for one is not one."""

    def _signed_doc(self):
        data = build_pdf_with_every_kind_of_field()
        doc = pymupdf.open(stream=data, filetype="pdf")
        xref = field_named(doc, "firma")["xref"]
        sign_field(doc, xref)
        reopened = pymupdf.open(stream=doc.tobytes(), filetype="pdf")
        doc.close()
        return reopened

    def test_an_unsigned_field_is_not_reported(self):
        doc = pymupdf.open(stream=build_pdf_with_every_kind_of_field(), filetype="pdf")
        assert signed_field_names(doc) == []
        doc.close()

    def test_a_signed_field_is_named(self):
        doc = self._signed_doc()
        assert signed_field_names(doc) == ["Cesar Ruiz"]
        doc.close()

    def test_a_document_with_no_form_reports_nothing(self):
        doc = pymupdf.open(stream=build_pdf(), filetype="pdf")
        assert signed_field_names(doc) == []
        doc.close()

    def test_it_is_carried_in_the_document_state(self):
        local = DocumentStore()
        document = local.open(self._signed_doc().tobytes(), "firmado.pdf")
        try:
            assert document.state()["signed_fields"] == ["Cesar Ruiz"]
        finally:
            local.close_all()

    def test_an_unsigned_document_reports_no_warning(self):
        local = DocumentStore()
        document = local.open(build_pdf_with_every_kind_of_field(), "formulario.pdf")
        try:
            assert document.state()["signed_fields"] == []
        finally:
            local.close_all()

    def test_the_warning_survives_an_unrelated_edit(self):
        """The signature is invalid the moment anything changes, not just at
        the instant it is edited — the banner has to keep saying so."""
        local = DocumentStore()
        document = local.open(self._signed_doc().tobytes(), "firmado.pdf")
        try:
            document.snapshot([0])
            apply_operations(document.doc, document.resolver, [{
                "op": "add_text", "page": 0, "rect": [72, 600, 300, 630],
                "text": "texto nuevo", "size": 11, "font": "helv",
            }])
            assert document.state()["signed_fields"] == ["Cesar Ruiz"]
        finally:
            local.close_all()

    def test_it_falls_back_to_the_fields_own_name(self):
        """A signature that carries no signer name is not silently dropped."""
        doc = pymupdf.open(stream=build_pdf_with_every_kind_of_field(), filetype="pdf")
        xref = field_named(doc, "firma")["xref"]
        sign_field(doc, xref, name=None)
        reopened = pymupdf.open(stream=doc.tobytes(), filetype="pdf")
        doc.close()
        assert signed_field_names(reopened) == ["firma"]
        reopened.close()
