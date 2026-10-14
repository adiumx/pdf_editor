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
lib.pdfcore_save.restype = ctypes.c_void_p
lib.pdfcore_save.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t)]
for name, args in {
    "pdfcore_page": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_fields": [ctypes.c_void_p],
    "pdfcore_replace_text": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_int],
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
