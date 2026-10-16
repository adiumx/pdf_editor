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
lib.pdfcore_move_field.restype = ctypes.c_void_p
lib.pdfcore_move_field.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int] + [ctypes.c_float] * 4
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
    "pdfcore_arrange_fields": [ctypes.c_void_p, ctypes.c_char_p],
    "pdfcore_items": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_move_items": [ctypes.c_void_p, ctypes.c_int, ctypes.c_char_p],
    "pdfcore_arrange_items": [ctypes.c_void_p, ctypes.c_int, ctypes.c_char_p],
    "pdfcore_space_lines": [ctypes.c_void_p, ctypes.c_int, ctypes.c_char_p],
    "pdfcore_align_paragraph": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int],
    "pdfcore_set_fields_alignment": [ctypes.c_void_p, ctypes.c_char_p],
    "pdfcore_required_fonts": [ctypes.c_void_p],
    "pdfcore_register_font": [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t],
    "pdfcore_replace_block_ex": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_char_p],
    "pdfcore_replace_range": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_char_p],
    "pdfcore_set_block_style": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_char_p],
    "pdfcore_block_chars": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int],
    "pdfcore_hit_test": [ctypes.c_void_p, ctypes.c_int, ctypes.c_float, ctypes.c_float],
    "pdfcore_delete_items": [ctypes.c_void_p, ctypes.c_int, ctypes.c_char_p],
    "pdfcore_resize_item": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_float, ctypes.c_float, ctypes.c_float, ctypes.c_float],
    "pdfcore_refresh_appearances": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_batch": [ctypes.c_void_p, ctypes.c_char_p],
    "pdfcore_annotations": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_add_annotation": [ctypes.c_void_p, ctypes.c_int, ctypes.c_char_p],
    "pdfcore_delete_annotation": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int],
    "pdfcore_set_annotation_contents": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_char_p],
    "pdfcore_move_annotation": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_float, ctypes.c_float],
    "pdfcore_flatten": [ctypes.c_void_p, ctypes.c_char_p],
    "pdfcore_redact": [ctypes.c_void_p, ctypes.c_int, ctypes.c_char_p],
    "pdfcore_ocr_report": [ctypes.c_void_p],
    "pdfcore_set_substitution": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_blocks": [ctypes.c_void_p, ctypes.c_int],
    "pdfcore_replace_block": [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_int],
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


def test_blocks_keep_the_style_of_each_word():
    import pymupdf
    d = Doc("producer_reportlab_ttf")
    blocks = call("pdfcore_blocks", d.handle, 0)["blocks"]
    mixed = next(b for b in blocks if b["text"].startswith("Para consultar"))
    assert mixed["justified"] and len(mixed["lines"]) == 4 and mixed["editable"]
    bold = [mixed["text"][s["start"]:s["end"]] for s in mixed["spans"] if s["bold"]]
    assert bold[-1] == "55 893 24827"
    text = mixed["text"].replace("55 893 24827", "55 111 22233 y 9").encode()
    done = call("pdfcore_replace_block", d.handle, 0, mixed["id"], text, 0)
    assert done["ok"] and done["rewritten"] >= 1, done
    # Ids are stale after the edit.
    assert call("pdfcore_replace_block", d.handle, 0, mixed["id"], b"x", 0)["kind"] == "stale_block"
    # An independent reader sees the new text, in bold where it was bold.
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    assert "55 111 22233 y 9" in " ".join(page.get_text().split())
    spans = [sp for b in page.get_text("dict")["blocks"] for l in b.get("lines", []) for sp in l["spans"]]
    assert any("Bold" in sp["font"] and "22233" in sp["text"] for sp in spans)
    d.close()


def test_a_moved_field_is_drawn_where_it_was_put():
    # Rendered, on a page that is turned: the text has to be inside the new box and gone from the old.
    for fixture in ("form", "form_rotated"):
        d = Doc(fixture)
        d.field("nombre", "XXXXXXXXXXXX")
        old = d.fields()["nombre"]["widgets"][0]["display"]
        done = call("pdfcore_move_field", d.handle, b"nombre", 0, 150.0, 400.0, 330.0, 430.0)
        assert done["ok"] and done["display"] == [150.0, 400.0, 330.0, 430.0], done
        pix = render(d.save())

        def ink(box):
            x0, y0, x1, y1 = (int(v) for v in box)
            return sum(1 for x in range(x0, x1) for y in range(y0, y1) if sum(pix.pixel(x, y)) < 300)

        assert ink((150, 400, 330, 430)) > 40, fixture
        # Upright: the ink is wider than it is tall, on a turned page too.
        dark = [(x, y) for x in range(150, 330) for y in range(400, 430) if sum(pix.pixel(x, y)) < 300]
        xs, ys = [p[0] for p in dark], [p[1] for p in dark]
        assert max(xs) - min(xs) > 3 * (max(ys) - min(ys)), (fixture, (min(xs), min(ys), max(xs), max(ys)))
        assert ink(tuple(old)) == 0 or tuple(old) == (150.0, 400.0, 330.0, 430.0), fixture
        d.close()


def test_arranging_fields_crosses_the_boundary():
    d = Doc("form_hard")
    fields = [{"name": n, "widget": 0} for n in ("rfc", "notas", "importe")]
    done = call("pdfcore_arrange_fields", d.handle, json.dumps({"fields": fields, "how": "align_left"}).encode())
    assert done == {"ok": True}
    lefts = {d.fields()[n]["widgets"][0]["display"][0] for n in ("rfc", "notas", "importe")}
    assert len(lefts) == 1
    assert call("pdfcore_arrange_fields", d.handle, json.dumps({"fields": fields, "how": "nope"}).encode())["kind"] == "bad_argument"
    assert call("pdfcore_arrange_fields", d.handle, json.dumps({"fields": fields[:1], "how": "align_left"}).encode())["kind"] == "rejected"
    d.close()


def test_items_move_and_line_up_and_are_checked_by_rendering():
    d = Doc("canvas")
    items = call("pdfcore_items", d.handle, 0)["items"]
    by_text = {i["text"]: i for i in items if i["kind"] == "text"}
    ids = [by_text[t]["id"] for t in ("Uno", "Dos mas largo", "Tres")]
    # Right-align the three texts: their right edges must meet on the rendered page.
    done = call("pdfcore_arrange_items", d.handle, 0, json.dumps({"ids": ids, "how": "align_right"}).encode())
    assert done == {"ok": True}, done
    pix = render(d.save())

    def right_edge(y0, y1):
        xs = [x for x in range(0, 300) for y in range(y0, y1) if sum(pix.pixel(x, y)) < 300]
        return max(xs)

    edges = [right_edge(y - 12, y + 4) for y in (110, 150, 190)]
    assert max(edges) - min(edges) <= 2, edges
    # Ids are stale after an edit.
    assert call("pdfcore_arrange_items", d.handle, 0, json.dumps({"ids": ids, "how": "align_left"}).encode())["kind"] == "stale_item"
    # A text moved down by 30 points appears 30 points lower.
    items = call("pdfcore_items", d.handle, 0)["items"]
    uno = next(i for i in items if i["text"] == "Uno")
    assert call("pdfcore_move_items", d.handle, 0, json.dumps({"moves": [{"id": uno["id"], "dx": 0, "dy": 30}]}).encode())["ok"]
    after = next(i for i in call("pdfcore_items", d.handle, 0)["items"] if i["text"] == "Uno")
    assert abs(after["display"][1] - uno["display"][1] - 30) < 0.1
    d.close()


def test_text_alignment_in_paragraphs_and_fields_crosses_the_boundary():
    d = Doc("canvas")
    runs = d.page()["runs"]
    first = next(i for i, r in enumerate(runs) if r["text"].startswith("Primera"))
    assert call("pdfcore_align_paragraph", d.handle, 0, first, 1)["ok"]
    assert call("pdfcore_align_paragraph", d.handle, 0, first, 1)["kind"] == "stale_run"
    done = call("pdfcore_set_fields_alignment", d.handle, json.dumps({"names": ["dato"], "align": "center"}).encode())
    assert done == {"ok": True}
    assert call("pdfcore_set_fields_alignment", d.handle, json.dumps({"names": ["dato"], "align": "middle"}).encode())["kind"] == "bad_argument"
    d.close()


# -- font fidelity ---------------------------------------------------------------

def oracle_text(data):
    import pymupdf, unicodedata
    return " ".join(unicodedata.normalize("NFC", pymupdf.open(stream=data, filetype="pdf")[0].get_text()).split())


def test_runs_say_how_their_font_can_be_reproduced():
    expected = {
        "standard": ("standard", False, True), "font_nonstd_notembedded": ("not_embedded", False, True),
        "embedded": ("embedded_subset", True, True), "font_tt_full_type0": ("embedded_full", True, True),
        "font_type3": ("type3", False, False), "font_zapf": ("standard", False, False),
        "font_type0_identity_v": ("embedded_subset", True, False), "font_rtl_arabic": ("embedded_full", True, False),
    }
    for fixture, (kind, embedded, editable) in expected.items():
        d = Doc(fixture)
        run = d.page()["runs"][0]
        assert (run["font_kind"], run["font_embedded"], run["editable"]) == (kind, embedded, editable), fixture
        assert (run["not_editable"] is None) == editable, fixture
        d.close()


def test_what_cannot_be_edited_gives_a_stable_kind_and_reason():
    for fixture, reason in (("font_type3", "type3"), ("font_type0_identity_v", "vertical_writing"),
                            ("font_zapf", "symbolic_font"), ("font_rtl_arabic", "rtl_text")):
        d = Doc(fixture)
        d.page()
        reply = d.replace_ex(0, "Hola", 1, 1)
        assert reply["kind"] == "not_editable" and reply["reason"] == reason and "error" in reply, (fixture, reply)
        assert call("pdfcore_history", d.handle)["can_undo"] is False, fixture   # a refusal is not a step
        d.close()
    # Replace-all leaves such text alone and says so.
    d = Doc("font_type3")
    done = call("pdfcore_replace_all", d.handle, b"AB", b"XY", 1, 0, 1)
    assert done == {"ok": True, "replaced": 0, "skipped": 2}, done
    assert call("pdfcore_history", d.handle)["can_undo"] is False
    d.close()


def test_encodings_agree_with_an_independent_reader_before_and_after_editing():
    for fixture, new_text in (("font_macroman", "Camión ñandú ÁÉÍÓÚ"), ("font_standardenc", "ﬁnal ﬂor"),
                              ("font_symbol", "αβ π"), ("font_type1_noenc", "Hola æ")):
        d = Doc(fixture)
        engine = " ".join(r["text"] for r in d.page()["runs"])
        import unicodedata
        assert unicodedata.normalize("NFC", engine) == oracle_text(d.save()), fixture
        reply = d.replace_ex(0, new_text, 0, 0)
        assert "error" not in reply and not reply["fallback_chars"], (fixture, reply)
        assert oracle_text(d.save()) == unicodedata.normalize("NFC", new_text), (fixture, oracle_text(d.save()))
        d.close()


def test_a_standard_font_keeps_its_face_for_a_new_letter():
    import pymupdf
    d = Doc("font_standardenc")
    d.page()
    done = d.replace(0, "mañana")
    assert done["ok"] and done["fallback_chars"] == ""
    assert done["substitutes"] == [{"char": "ñ", "via": "encoding", "font": "Helvetica", "similarity": 100}], done
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    spans = [sp for b in page.get_text("dict")["blocks"] for l in b["lines"] for sp in l["spans"]]
    assert "mañana" in " ".join(sp["text"] for sp in spans)
    # An independent renderer draws ink where the letter is (not a box, not blank).
    clip = page.search_for("mañana")[0]
    pix = page.get_pixmap(clip=clip, dpi=200)
    assert sum(1 for i in range(0, len(pix.samples), pix.n) if pix.samples[i] < 128) > 50
    d.close()


def test_substitution_policy_draws_the_whole_run_in_one_face():
    import pymupdf
    d = Doc("font_subset_as_verdana")
    assert call("pdfcore_set_substitution", d.handle, 1)["ok"]
    d.page()
    done = d.replace(0, "Hola mundo cruel de Ñandú", fallback=1)
    assert done["ok"] and len(done["substitutes"]) == 25, done
    assert {s["via"] for s in done["substitutes"]} == {"bundled_font"}
    assert all(s["similarity"] < 90 and s["font"].startswith("OpenSans") for s in done["substitutes"])
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    assert "Ñandú" in page.get_text()
    spans = [sp for b in page.get_text("dict")["blocks"] for l in b["lines"] for sp in l["spans"]]
    assert all("Liberation" in sp["font"] or "Sans" in sp["font"] for sp in spans), [sp["font"] for sp in spans]
    d.close()


def test_required_fonts_and_a_registered_font():
    d = Doc("font_subset_as_verdana")
    fonts = call("pdfcore_required_fonts", d.handle)["fonts"]
    assert fonts[0]["base_font"] == "ABCDEF+Verdana" and fonts[0]["needed"] and not fonts[0]["registered"]
    assert fonts[0]["source"]["name"] == "Open Sans (bundled)" and fonts[0]["similarity"] < 90
    run = d.page()["runs"][0]
    assert run["substitute_similarity"] < 90
    data = (FIXTURES / "registered_verdana.ttf").read_bytes()
    done = call("pdfcore_register_font", d.handle, data, len(data))
    assert done == {"ok": True, "family": "Verdana", "bold": False, "italic": False}, done
    assert call("pdfcore_required_fonts", d.handle)["fonts"][0]["registered"]
    assert d.page()["runs"][0]["substitute_similarity"] == 100
    assert call("pdfcore_register_font", d.handle, b"nope", 4)["kind"] == "unsupported"
    d.close()


def test_block_styles_cross_the_boundary_and_an_independent_reader_agrees():
    import pymupdf
    d = Doc("producer_reportlab_ttf")
    mixed = next(b for b in call("pdfcore_blocks", d.handle, 0)["blocks"] if b["text"].startswith("Para consultar"))
    done = call("pdfcore_set_block_style", d.handle, 0, mixed["id"], 0, 4, json.dumps({"bold": True, "color": [1, 0, 0]}).encode())
    assert done["ok"], done
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    spans = [sp for b in page.get_text("dict")["blocks"] for l in b["lines"] for sp in l["spans"]]
    first = next(sp for sp in spans if sp["text"].startswith("Para"))
    assert "Bold" in first["font"] and first["color"] == 0xFF0000, first
    # Ids are stale after the edit; a fresh reading has boxes for every letter.
    assert call("pdfcore_set_block_style", d.handle, 0, mixed["id"], 0, 4, b"{}")["kind"] == "stale_block"
    mixed = next(b for b in call("pdfcore_blocks", d.handle, 0)["blocks"] if b["text"].startswith("Para consultar"))
    boxes = call("pdfcore_block_chars", d.handle, 0, mixed["id"])["boxes"]
    assert len(boxes) == len(mixed["text"])
    other = next(b for b in call("pdfcore_blocks", d.handle, 0)["blocks"] if b["text"].startswith("Otro"))
    longer = (mixed["text"] + " Mas texto para que el parrafo crezca. " * 8).encode()
    done = call("pdfcore_replace_block_ex", d.handle, 0, mixed["id"], longer, json.dumps({"push_down": True, "justify": False}).encode())
    assert done["ok"] and done["added"] >= 1 and done["warnings"] == [], done
    moved = next(b for b in call("pdfcore_blocks", d.handle, 0)["blocks"] if b["text"].startswith("Otro"))
    assert moved["display"][1] > other["display"][1] + 10
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    text = " ".join(page.get_text().split())
    assert "Otro párrafo normal" in text and "Mas texto para que el parrafo crezca" in text
    d.close()


def test_hit_delete_resize_and_batch_cross_the_boundary():
    import pymupdf
    d = Doc("canvas")
    items = call("pdfcore_items", d.handle, 0)["items"]
    text = next(i for i in items if i["kind"] == "text" and i["text"].startswith("Primera"))
    cx, cy = (text["display"][0] + text["display"][2]) / 2, (text["display"][1] + text["display"][3]) / 2
    hit = call("pdfcore_hit_test", d.handle, 0, cx, cy)["hit"]
    assert hit["kind"] == "text" and hit["char"] is not None and hit["block"] is not None, hit
    assert call("pdfcore_hit_test", d.handle, 0, 590.0, 835.0)["hit"] is None

    # A batch: move a text, resize a picture and set a field: ONE undo step takes all of it back.
    items = call("pdfcore_items", d.handle, 0)["items"]
    pic = next(i for i in items if i["kind"] == "image")
    field = next(i for i in items if i["kind"] == "field")
    before = d.save()
    ops = {"ops": [
        {"op": "move_items", "page": 0, "moves": [{"id": text["id"], "dx": 5, "dy": 6}]},
        {"op": "resize_item", "page": 0, "id": pic["id"], "box": [pic["display"][0], pic["display"][1], pic["display"][0] + 30, pic["display"][1] + 30]},
        {"op": "set_field", "name": field["name"], "value": "Nuevo valor"},
    ]}
    done = call("pdfcore_batch", d.handle, json.dumps(ops).encode())
    assert done["ok"] and len(done["results"]) == 3, done
    assert d.fields()[field["name"]]["value"] == "Nuevo valor"
    assert d.undo()["ok"]
    assert d.save() == before

    # A failing op undoes the ones before it.
    items = call("pdfcore_items", d.handle, 0)["items"]
    bad = {"ops": [
        {"op": "set_field", "name": field["name"], "value": "Otro"},
        {"op": "set_field", "name": "no existe", "value": "x"},
    ]}
    failed = call("pdfcore_batch", d.handle, json.dumps(bad).encode())
    assert failed["kind"] == "no_such_field", failed
    assert d.save() == before

    # Delete: the picture is gone for an independent reader too.
    items = call("pdfcore_items", d.handle, 0)["items"]
    pics = [i["id"] for i in items if i["kind"] == "image"]
    assert call("pdfcore_delete_items", d.handle, 0, json.dumps({"ids": pics}).encode())["ok"]
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    assert page.get_images() == [] or all(page.get_image_rects(img[0]) == [] for img in page.get_images())
    d.close()


def test_missing_looks_are_rebuilt():
    import pymupdf
    d = Doc("form_no_appearance")
    done = call("pdfcore_refresh_appearances", d.handle, 1)
    assert done["ok"] and done["rebuilt"] >= 1, done
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    assert all(w.field_name for w in page.widgets())
    d.close()


def test_annotations_are_seen_by_an_independent_reader():
    import pymupdf
    d = Doc("standard")
    box = d.page()["runs"][0]["display"]
    add = lambda p: call("pdfcore_add_annotation", d.handle, 0, json.dumps(p).encode())
    assert add({"shape": "highlight", "boxes": [box], "color": [1, 0.9, 0.2]})["ok"]
    assert add({"shape": "note", "at": [300, 100], "contents": "Revisar", "author": "Cesar"})["ok"]
    assert add({"shape": "free_text", "rect": [100, 300, 300, 360], "size": 12, "contents": "Texto libre con acentos: ñandú"})["ok"]
    assert add({"shape": "ink", "strokes": [[[100, 400], [150, 420], [200, 400]]], "width": 2, "color": [1, 0, 0]})["ok"]
    assert add({"shape": "square", "rect": [100, 450, 200, 500], "fill": [0.8, 0.9, 1], "width": 1.5, "color": [0, 0, 1]})["ok"]
    assert add({"shape": "circle", "rect": [220, 450, 320, 500], "width": 1.5, "color": [0, 0.5, 0]})["ok"]
    assert add({"shape": "line", "from": [100, 520], "to": [300, 560], "width": 2, "color": [1, 0, 0]})["ok"]
    assert add({"shape": "free_text", "rect": [100, 300, 300, 360], "contents": "\u4e16"})["kind"] == "missing_glyphs"
    listed = call("pdfcore_annotations", d.handle, 0)["annotations"]
    assert [a["kind"] for a in listed] == ["highlight", "note", "free_text", "ink", "square", "circle", "line"]
    data = d.save()
    page = pymupdf.open(stream=data, filetype="pdf")[0]
    assert [a.type[1] for a in page.annots()] == ["Highlight", "Text", "FreeText", "Ink", "Square", "Circle", "Line"]
    # The highlight multiplies yellow over the text's own box; the free text is there for a text reader.
    pix = page.get_pixmap(clip=pymupdf.Rect(box[0], box[1], box[2], box[3]), dpi=72)
    px = pix.pixel(1, 1)
    assert px[0] > 200 and px[1] > 180 and px[2] < 120, px
    assert "ñandú" in " ".join(page.get_text().split()) or any("ñandú" in (a.info.get("content") or "") for a in page.annots())
    # Move, re-word, delete.
    assert call("pdfcore_move_annotation", d.handle, 0, 1, 10.0, 20.0)["ok"]
    assert call("pdfcore_set_annotation_contents", d.handle, 0, 2, "Otro texto".encode())["ok"]
    assert call("pdfcore_delete_annotation", d.handle, 0, 0)["ok"]
    after = call("pdfcore_annotations", d.handle, 0)["annotations"]
    assert len(after) == 6 and after[0]["kind"] == "note" and after[1]["contents"] == "Otro texto"
    d.close()


def test_flatten_and_redact_agree_with_an_independent_reader():
    import pymupdf
    d = Doc("form")
    name = next(n for n, f in d.fields().items() if f["kind"] == "text" and not f["read_only"])
    assert d.field(name, "Valor aplanado")["ok"]
    done = call("pdfcore_flatten", d.handle, json.dumps({"annotations": True, "forms": True}).encode())
    assert done["ok"] and done["drawn"] >= 1, done
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    assert "Valor aplanado" in page.get_text()
    assert [w.field_name for w in page.widgets()] == ["firma"]
    d.close()

    d = Doc("producer_reportlab_ttf")
    reader = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    words = reader.get_text("words")
    target = next(w for w in words if w[4] == "horarios")
    neighbours = {w[4]: w[:4] for w in words if w[1] == target[1] and w[4] in ("días", "atención;")}
    box = [target[0] + 0.3, target[1], target[2] - 0.3, target[3]]
    done = call("pdfcore_redact", d.handle, 0, json.dumps({"boxes": [box]}).encode())
    assert done["ok"] and done["chars"] == len("horarios"), done
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    text = page.get_text()
    assert "horarios" not in text
    after = {w[4]: w[:4] for w in page.get_text("words")}
    for word, old in neighbours.items():
        assert all(abs(a - b) < 0.7 for a, b in zip(after[word], old)), (word, old, after[word])
    # Covered: black where the word was.
    pix = page.get_pixmap(clip=pymupdf.Rect(*box), dpi=72)
    assert pix.pixel(int((box[2] - box[0]) / 2), int((box[3] - box[1]) / 2))[0] < 40
    d.close()


def test_ocr_report_and_incremental_save():
    import pymupdf
    d = Doc("scan_no_text")
    pages = call("pdfcore_ocr_report", d.handle)["pages"]
    assert pages[0]["kind"] == "scanned_no_text" and pages[0]["needs_ocr"]
    d.close()
    original = (FIXTURES / "standard.pdf").read_bytes()
    d = Doc("standard")
    d.page()
    assert d.replace(0, "Incremental")["ok"]
    size = ctypes.c_size_t()
    lib.pdfcore_save_incremental.restype = ctypes.c_void_p
    lib.pdfcore_save_incremental.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t)]
    ptr = lib.pdfcore_save_incremental(d.handle, ctypes.byref(size))
    assert ptr and size.value > len(original)
    saved = ctypes.string_at(ptr, size.value)
    lib.pdfcore_free_bytes(ctypes.c_void_p(ptr), size.value)
    assert saved[:len(original)] == original
    assert "Incremental" in pymupdf.open(stream=saved, filetype="pdf")[0].get_text()
    d.close()


def test_a_cut_down_bundled_font_still_draws_its_letters():
    import pymupdf
    d = Doc("embedded")
    d.page()
    done = d.replace(0, "Ж Щ Ñ", fallback=1)
    assert done["ok"] and sorted(done["fallback_chars"]) == sorted("ЖЩÑ") , done
    page = pymupdf.open(stream=d.save(), filetype="pdf")[0]
    assert "Ж Щ Ñ" in " ".join(page.get_text().split())
    hits = page.search_for("Ж")
    assert hits
    pix = page.get_pixmap(clip=hits[0], dpi=200)
    ink = sum(1 for i in range(0, len(pix.samples), pix.n) if pix.samples[i] < 128)
    assert ink > 80, ink
    d.close()


def test_inline_protection_form_xobject_text_and_the_independent_reader():
    import pymupdf
    # MuPDF's own way of writing /Encrypt opens now, and the edit survives a round trip.
    d = Doc("encrypted_inline")
    d.page()
    assert d.replace(0, "Editado")["ok"]
    assert "Editado" in pymupdf.open(stream=d.save(), filetype="pdf")[0].get_text()
    d.close()
    # Text drawn by a form XObject is edited in the form itself.
    d = Doc("line_xobject")
    try:
        done = d.replace(run_named(d, "lunes a viernes")["id"], "otra cosa")
        assert done["ok"], done
        assert "otra cosa" in pymupdf.open(stream=d.save(), filetype="pdf")[0].get_text()
    finally:
        d.close()
