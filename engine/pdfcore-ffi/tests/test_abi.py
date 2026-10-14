"""The C interface, called the way Dart's FFI will: through the shared library.

    cargo build -p pdfcore-ffi && python3 -m pytest pdfcore-ffi/tests
"""
import ctypes, json, pathlib

import pytest

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "pdfcore/tests/fixtures"
LIB = ROOT / "target/debug/libpdfcore_ffi.so"

lib = ctypes.CDLL(str(LIB))
lib.pdfcore_open.restype = ctypes.c_void_p
lib.pdfcore_open.argtypes = [ctypes.c_char_p, ctypes.c_size_t]
lib.pdfcore_last_error.restype = ctypes.c_char_p
lib.pdfcore_close.argtypes = [ctypes.c_void_p]
lib.pdfcore_free_string.argtypes = [ctypes.c_void_p]
lib.pdfcore_free_bytes.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
lib.pdfcore_page_count.argtypes = [ctypes.c_void_p]
lib.pdfcore_open_with_password.restype = ctypes.c_void_p
lib.pdfcore_open_with_password.argtypes = [ctypes.c_char_p, ctypes.c_size_t, ctypes.c_char_p]
lib.pdfcore_last_error_kind.restype = ctypes.c_char_p
lib.pdfcore_save.restype = ctypes.c_void_p
lib.pdfcore_save.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t)]
for name, args in {
    "pdfcore_page": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_fields": [ctypes.c_void_p],
    "pdfcore_replace_text": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_int],
    "pdfcore_replace_text_ex": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_int],
    "pdfcore_signatures": [ctypes.c_void_p],
    "pdfcore_add_text": [ctypes.c_void_p, ctypes.c_int, ctypes.c_char_p],
    "pdfcore_set_field": [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_int],
    "pdfcore_undo": [ctypes.c_void_p], "pdfcore_redo": [ctypes.c_void_p], "pdfcore_history": [ctypes.c_void_p],
}.items():
    fn = getattr(lib, name)
    fn.restype = ctypes.c_void_p      # not c_char_p: that would copy and lose the pointer to free
    fn.argtypes = args


def call(name, *args):
    raw = getattr(lib, name)(*args)
    assert raw, f"{name} devolvió NULL"
    try:
        return json.loads(ctypes.string_at(raw).decode())
    finally:
        lib.pdfcore_free_string(raw)


class Doc:
    def __init__(self, source):
        data = source if isinstance(source, bytes) else (FIXTURES / f"{source}.pdf").read_bytes()
        self.handle = lib.pdfcore_open(data, len(data))
        assert self.handle, lib.pdfcore_last_error()

    def page(self, n=0): return call("pdfcore_page", self.handle, n)
    def texts(self, n=0): return [r["text"] for r in self.page(n)["runs"]]
    def replace(self, run, text, fallback=0, page=0): return call("pdfcore_replace_text", self.handle, page, run, text.encode(), fallback)
    def replace_ex(self, run, text, reflow, fallback=0, page=0): return call("pdfcore_replace_text_ex", self.handle, page, run, text.encode(), fallback, reflow)
    def signatures(self): return call("pdfcore_signatures", self.handle)["signatures"]
    def add(self, params, page=0): return call("pdfcore_add_text", self.handle, page, json.dumps(params).encode())
    def field(self, name, value, force=0): return call("pdfcore_set_field", self.handle, name.encode(), value.encode(), force)
    def fields(self): return {f["name"]: f for f in call("pdfcore_fields", self.handle)["fields"]}
    def undo(self): return call("pdfcore_undo", self.handle)
    def redo(self): return call("pdfcore_redo", self.handle)

    def save(self):
        size = ctypes.c_size_t()
        ptr = lib.pdfcore_save(self.handle, ctypes.byref(size))
        assert ptr and size.value > 0
        try:
            return ctypes.string_at(ptr, size.value)
        finally:
            lib.pdfcore_free_bytes(ptr, size.value)

    def close(self): lib.pdfcore_close(self.handle)


@pytest.fixture
def doc():
    d = Doc("standard")
    yield d
    d.close()


def test_open_count_and_read(doc):
    assert lib.pdfcore_page_count(doc.handle) == 1
    page = doc.page()
    assert (page["width"], page["height"], page["rotation"]) == (595.0, 842.0, 0)
    first = page["runs"][0]
    assert first["text"] == "Hola mundo" and first["id"] == 0 and first["base_font"] == "Helvetica"
    assert len(first["display"]) == 4 and first["invisible"] is False


def test_a_bad_file_fails_cleanly_with_a_reason():
    handle = lib.pdfcore_open(b"esto no es un pdf", 17)
    assert not handle
    assert lib.pdfcore_last_error().decode()


def test_null_and_nonsense_arguments_do_not_crash():
    assert call("pdfcore_page", None, 0)["kind"] == "bad_argument"
    assert call("pdfcore_undo", None)["kind"] == "bad_argument"
    assert lib.pdfcore_page_count(None) == -1
    assert not lib.pdfcore_open(None, 0)
    lib.pdfcore_close(None)
    lib.pdfcore_free_string(None)


def test_replace_text_and_the_ids_it_invalidates(doc):
    doc.page()
    done = doc.replace(0, "Adiós")
    assert done["ok"] is True and done["new_width"] < done["old_width"]
    # The page was not read again since the edit: the old id means nothing now.
    assert doc.replace(0, "otra vez")["kind"] == "stale_run"
    assert doc.texts()[0] == "Adiós"      # reading the page renews the ids
    assert doc.replace(0, "otra vez")["ok"] is True


def test_missing_glyphs_name_the_characters_and_fallback_fixes_it():
    d = Doc("embedded")
    try:
        d.page()
        refused = d.replace(0, "Póliza XQ")
        assert refused["kind"] == "missing_glyphs" and refused["chars"] == "XQ"
        assert d.texts() == ["Póliza número 12345"]
        d.page()
        done = d.replace(0, "Póliza XQ", fallback=1)
        assert done["ok"] and done["fallback_chars"] == "XQ" and done["reread"] is True
        assert "".join(d.texts()) == "Póliza XQ"
    finally:
        d.close()


def test_add_text_with_options(doc):
    done = doc.add({"text": "Nombre completo", "x": 72, "y": 300, "size": 14, "bold": True, "color": [0, 0, 1]})
    assert done["ok"] and done["lines"] == 1
    assert doc.texts()[-1] == "Nombre completo"
    assert doc.page()["runs"][-1]["base_font"] == "LiberationSans-Bold"
    assert doc.add({"text": "x"})["kind"] == "bad_argument"
    assert doc.add({"text": "世界", "x": 1, "y": 1})["kind"] == "missing_glyphs"


def test_fields_fill_and_refuse():
    d = Doc("form")
    try:
        fields = d.fields()
        assert fields["nombre"]["value"] == "Ana" and fields["fijo"]["read_only"] is True
        assert fields["pais"]["kind"] == "combo" and len(fields["pais"]["options"]) == 3
        assert d.field("nombre", "María")["ok"]
        assert d.field("corto", "demasiado largo")["kind"] == "rejected"
        assert d.field("fijo", "x")["kind"] == "rejected"
        assert d.field("fijo", "x", force=1)["ok"]
        assert d.field("nada", "x")["kind"] == "no_such_field"
        assert d.fields()["nombre"]["value"] == "María"
    finally:
        d.close()


def test_undo_redo_and_saving_round_trip(doc):
    doc.page()
    doc.replace(0, "Cambiado")
    doc.add({"text": "extra", "x": 72, "y": 400})
    saved = doc.save()
    assert saved.startswith(b"%PDF")

    again = Doc(saved)
    try:
        assert again.texts() == ["Cambiado", "Negrita áéíóú ñ", "extra"]
    finally:
        again.close()

    assert doc.undo()["ok"] and doc.undo()["can_redo"] is True
    assert doc.texts() == ["Hola mundo", "Negrita áéíóú ñ"]
    assert doc.undo()["ok"] is False
    assert doc.redo()["ok"] and doc.redo()["ok"]
    assert doc.texts()[0] == "Cambiado"


def test_a_document_can_be_opened_many_times_and_closed():
    for _ in range(50):
        d = Doc("standard")
        d.page()
        d.close()


def run_named(d, text):
    return next(r for r in d.page()["runs"] if r["text"].strip() == text)


def test_a_longer_run_moves_the_rest_of_its_line_and_says_how_many():
    d = Doc("line_absolute")
    try:
        after = run_named(d, "de 9 a 14 horas")["origin"][0]
        first = run_named(d, "lunes a viernes")
        done = d.replace(first["id"], "de lunes a viernes inclusive")
        assert done["ok"] and done["moved"] == 1 and done["warnings"] == []
        assert done["reread"] is True          # a move was inserted: ids renumbered
        shifted = run_named(d, "de 9 a 14 horas")["origin"][0] - after
        assert abs(shifted - (done["new_width"] - done["old_width"])) < 0.05
        assert run_named(d, "Segunda linea del documento")["origin"] == [72.0, 680.0]
    finally:
        d.close()


def test_reflow_modes_and_warnings_cross_the_boundary():
    d = Doc("line_absolute")
    try:
        before = run_named(d, "de 9 a 14 horas")["origin"][0]
        done = d.replace_ex(run_named(d, "lunes a viernes")["id"], "de lunes a viernes inclusive", reflow=0)
        assert done["moved"] == 0
        assert run_named(d, "de 9 a 14 horas")["origin"][0] == before     # nothing moved
        d.undo()
        far = d.replace(run_named(d, "lunes a viernes")["id"], "lunes a viernes y tambien " * 6)
        assert any(w["kind"] == "exceeds_page" and w["by"] > 0 for w in far["warnings"])
    finally:
        d.close()


def test_a_follower_in_an_xobject_is_reported_not_moved():
    d = Doc("line_xobject")
    try:
        done = d.replace(run_named(d, "El resto de las oficinas de")["id"], "El resto de todas las oficinas de pasaportes de")
        assert {"kind": "follower_not_moved", "text": "lunes a viernes"} in done["warnings"]
    finally:
        d.close()


def test_signatures_and_appearance_flags():
    d = Doc("signed")
    try:
        assert d.signatures() == [{"field": "firma", "signer": "Cesar Ruiz"}]
        assert d.fields()["firma"]["signed"] is True
    finally:
        d.close()
    d = Doc("form_no_appearance")
    try:
        assert d.fields()["nombre"]["widgets"][0]["has_appearance"] is False
        d.field("nombre", "Con apariencia")
        assert d.fields()["nombre"]["widgets"][0]["has_appearance"] is True
    finally:
        d.close()
    d = Doc("form")
    try:
        assert d.signatures() == []
    finally:
        d.close()


def test_a_protected_document_asks_for_its_password():
    data = (FIXTURES / "encrypted_user.pdf").read_bytes()
    assert not lib.pdfcore_open(data, len(data))
    assert lib.pdfcore_last_error_kind() == b"password_required"
    assert not lib.pdfcore_open_with_password(data, len(data), b"mal")
    assert lib.pdfcore_last_error_kind() == b"wrong_password"
    handle = lib.pdfcore_open_with_password(data, len(data), b"secreto")
    assert handle and lib.pdfcore_page_count(handle) == 1
    lib.pdfcore_close(handle)
    # Only an owner password: opens with none.
    free = Doc("encrypted_owner")
    assert free.texts() == ["Hola mundo", "Negrita áéíóú ñ"]
    free.close()


def test_harder_forms_cross_the_boundary():
    d = Doc("form_hard")
    fields = d.fields()
    assert fields["ramos"]["multi_select"] is True
    assert [n for n, f in sorted(fields.items(), key=lambda kv: kv[1]["order"])][:3] == ["rfc", "notas", "importe"]
    assert "error" not in d.field("ramos", "Vida\nAuto")
    assert d.fields()["ramos"]["value"] == "Vida\nAuto"
    d.close()
