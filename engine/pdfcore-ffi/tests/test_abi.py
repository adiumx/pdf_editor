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
lib.pdfcore_insert_pages.restype = ctypes.c_void_p
lib.pdfcore_insert_pages.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t, ctypes.c_char_p,
                                     ctypes.POINTER(ctypes.c_int), ctypes.c_size_t, ctypes.c_int]
lib.pdfcore_add_image.restype = ctypes.c_void_p
lib.pdfcore_add_image.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_size_t, ctypes.c_char_p]
lib.pdfcore_extract_pages.restype = ctypes.c_void_p
lib.pdfcore_extract_pages.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int), ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)]
lib.pdfcore_save.restype = ctypes.c_void_p
lib.pdfcore_save.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t)]
for name, args in {
    "pdfcore_page": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_fields": [ctypes.c_void_p],
    "pdfcore_replace_text": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_int],
    "pdfcore_replace_text_ex": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_int],
    "pdfcore_signatures": [ctypes.c_void_p],
    "pdfcore_lines": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_paragraph": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int],
    "pdfcore_replace_paragraph": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_int],
    "pdfcore_rotate_page": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int],
    "pdfcore_delete_page": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_move_page": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int],
    "pdfcore_find": [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int],
    "pdfcore_replace_all": [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_int, ctypes.c_int, ctypes.c_int],
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


def test_lines_search_and_replace_all_cross_the_boundary():
    d = Doc("multi")
    lines = call("pdfcore_lines", d.handle, 0)["lines"]
    assert lines[0]["text"] == "Contrato 1 de la aseguradora Prudential"
    assert [w["text"] for w in lines[0]["words"]][-1] == "Prudential"
    # The run ids of a line are good for editing straight away.
    assert "error" not in d.replace(lines[0]["runs"][0], "Contrato uno de la aseguradora Prudential")
    hits = call("pdfcore_find", d.handle, b"nunez", 0)["matches"]
    assert len(hits) == 3 and hits[0]["within_run"] is not None
    done = call("pdfcore_replace_all", d.handle, b"Prudential", b"Metlife", 1, 0, 1)
    assert done["ok"] and done["replaced"] == 9 and done["skipped"] == 0, done
    assert call("pdfcore_find", d.handle, b"Prudential", 1)["matches"] == []
    assert call("pdfcore_replace_all", d.handle, b"", b"x", 0, 0, 1)["kind"] == "rejected"
    d.close()


def test_page_operations_cross_the_boundary():
    d = Doc("multi")
    assert call("pdfcore_rotate_page", d.handle, 0, 90)["ok"]
    assert d.page(0)["rotation"] == 90
    assert call("pdfcore_move_page", d.handle, 0, 2)["ok"]
    assert call("pdfcore_delete_page", d.handle, 0)["pages"] == 2
    assert call("pdfcore_rotate_page", d.handle, 0, 45)["kind"] == "rejected"
    other = (FIXTURES / "form.pdf").read_bytes()
    added = call("pdfcore_insert_pages", d.handle, other, len(other), None, None, 0, 1)
    assert added == {"ok": True, "added": 1, "pages": 3}
    # Extract pages 2 and 0 as a new document.
    pages = (ctypes.c_int * 2)(2, 0)
    size = ctypes.c_size_t()
    ptr = lib.pdfcore_extract_pages(d.handle, pages, 2, ctypes.byref(size))
    assert ptr and size.value > 0
    part = Doc(ctypes.string_at(ptr, size.value))
    lib.pdfcore_free_bytes(ptr, size.value)
    assert lib.pdfcore_page_count(part.handle) == 2
    part.close()
    bad = (ctypes.c_int * 1)(99)
    assert not lib.pdfcore_extract_pages(d.handle, bad, 1, ctypes.byref(size))
    assert lib.pdfcore_last_error_kind() == b"no_such_page"
    d.close()


# -- pictures, checked against a renderer ----------------------------------------

def render(doc_bytes, page=0):
    import pymupdf
    return pymupdf.open(stream=doc_bytes, filetype="pdf")[page].get_pixmap(alpha=False)


def place(d, picture, x, y, width, height=None, page=0):
    data = (FIXTURES / picture).read_bytes()
    params = {"x": x, "y": y, "width": width}
    if height:
        params["height"] = height
    return call("pdfcore_add_image", d.handle, page, data, len(data), json.dumps(params).encode())


def near(pixel, expected, tolerance=40):
    return all(abs(a - b) <= tolerance for a, b in zip(pixel, expected))


RED, BLUE, WHITE, GREEN = (255, 0, 0), (0, 0, 255), (255, 255, 255), (0, 160, 0)


def test_a_jpeg_lands_where_it_was_placed():
    d = Doc("standard")
    done = place(d, "img_rgb.jpg", 100, 200, 160)          # 160 x 80
    assert done["ok"] and done["display"] == [100, 200, 260, 280]
    pix = render(d.save())
    assert near(pix.pixel(110, 240), RED) and near(pix.pixel(250, 240), BLUE)
    assert near(pix.pixel(180, 215), RED) or near(pix.pixel(150, 215), RED)
    assert near(pix.pixel(95, 240), WHITE) and near(pix.pixel(265, 240), WHITE)
    assert near(pix.pixel(150, 195), WHITE) and near(pix.pixel(150, 285), WHITE)
    d.close()


def test_exif_orientation_is_applied_to_the_placement():
    # Stored 80x40, left half red, right half blue.
    cases = {
        "img_exif3.jpg": ("left blue, right red", (140, 220), BLUE, (140, 260), RED, 80, 40),   # 180 degrees
        "img_exif6.jpg": ("red on top", (140, 230), RED, (140, 350), BLUE, 40, 80),             # rotate 90 clockwise
        "img_exif8.jpg": ("red at the bottom", (140, 230), BLUE, (140, 350), RED, 40, 80),      # rotate 90 counter-clockwise
    }
    for name, (what, p1, c1, p2, c2, _, _) in cases.items():
        d = Doc("standard")
        done = place(d, name, 100, 200, 80)
        pix = render(d.save())
        if name == "img_exif3.jpg":
            p1, p2 = (110, 220), (170, 220)        # left half and right half of an 80x40 box
        assert near(pix.pixel(*p1), c1) and near(pix.pixel(*p2), c2), (name, what, done)
        d.close()


def test_pictures_land_right_on_a_turned_page_and_one_that_leaks_state():
    for fixture, size in (("page_rotated", None), ("leaky", None), ("rotated", None)):
        d = Doc(fixture)
        shown = d.page()
        # The box is given as a reader shows the page, whatever the page's own turn.
        place(d, "img_rgb.jpg", 20, 20, 100)             # 100 x 50
        pix = render(d.save())
        assert (pix.width, pix.height) == (round(shown["width"]), round(shown["height"])), fixture
        assert near(pix.pixel(40, 45), RED) and near(pix.pixel(100, 45), BLUE), fixture
        assert near(pix.pixel(10, 45), WHITE) and near(pix.pixel(40, 80), WHITE), fixture
        d.close()


def test_a_png_keeps_its_transparency():
    d = Doc("standard")
    place(d, "img_alpha.png", 100, 300, 80)               # 80 x 80: left half green, right half see-through
    pix = render(d.save())
    assert near(pix.pixel(120, 340), GREEN) and near(pix.pixel(170, 340), WHITE)
    d.close()


def test_paragraphs_cross_the_boundary():
    d = Doc("paragraph")
    first = next(i for i, r in enumerate(d.page()["runs"]) if r["text"].startswith("El asegurado"))
    title = next(i for i, r in enumerate(d.page()["runs"]) if r["text"].startswith("Condiciones"))
    paragraph = call("pdfcore_paragraph", d.handle, 0, first)["paragraph"]
    assert len(paragraph["runs"]) == 5 and paragraph["text"].startswith("El asegurado debe notificar")
    assert call("pdfcore_paragraph", d.handle, 0, title)["paragraph"] is None
    longer = (paragraph["text"] + " Ademas, debera entregar copia de cualquier documento que respalde el cambio.").encode()
    done = call("pdfcore_replace_paragraph", d.handle, 0, first, longer, 0)
    assert done["ok"] and done["added"] >= 1 and done["removed"] == 0 and done["warnings"] == [], done
    # Ids are stale after the edit, like any edit.
    assert call("pdfcore_replace_paragraph", d.handle, 0, first, b"x", 0)["kind"] == "stale_run"
    assert call("pdfcore_replace_paragraph", d.handle, 0, 999, b"x", 0)["kind"] == "stale_run"
    d.close()
