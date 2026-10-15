"""Phase 1B: does opening and saving change what a PDF looks like?

Every PDF is drawn by an independent renderer (MuPDF through PyMuPDF) before and after
a series of operations that rewrite the file but must not change the picture: saving
as it is, saving twice, turning a page and undoing, moving a page away and back, bringing
a page in and deleting it, and (informational) editing a run to the very same text.
Also compared: page count, extracted text, form fields and annotations.

    cargo build -p pdfcore-ffi && python3 tools/roundtrip_check.py [extra.pdf ...]
"""
import ctypes, glob, pathlib, sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "pdfcore-ffi/tests"))
import pymupdf
import test_abi as t

PASSWORDS = {"encrypted_user": "secreto"}
SKIP = {"encrypted_inline"}          # not readable by the engine (reported as unsupported)
TOLERANCE = 8                        # per colour channel, out of 255
DPI = 72


def open_doc(data, password=""):
    handle = t.lib.pdfcore_open_with_password(data, len(data), password.encode())
    if not handle:
        raise RuntimeError(t.lib.pdfcore_last_error().decode())
    doc = t.Doc.__new__(t.Doc)
    doc.handle = handle
    return doc


def pages(data, password=""):
    d = pymupdf.open(stream=data, filetype="pdf")
    if d.needs_pass:
        d.authenticate(password)
    return d


def different(a, b):
    """How many samples of two renderings differ by more than the tolerance (-1: different size)."""
    if (a.width, a.height) != (b.width, b.height):
        return -1
    if a.samples == b.samples:
        return 0
    return sum(1 for x, y in zip(a.samples, b.samples) if abs(x - y) > TOLERANCE)


def compare(original, saved, password=""):
    a, b = pages(original, password), pages(saved)
    if len(a) != len(b):
        return f"{len(a)} → {len(b)} páginas"
    bad = []
    for i in range(len(a)):
        diff = different(a[i].get_pixmap(dpi=DPI), b[i].get_pixmap(dpi=DPI))
        if diff:
            bad.append(f"p{i + 1}:{'tamaño' if diff < 0 else diff}")
    if bad:
        return "DIFIERE " + " ".join(bad[:4])
    extra = []
    if [p.get_text() for p in a] != [p.get_text() for p in b]:
        extra.append("texto")
    if [len(list(p.widgets() or [])) for p in a] != [len(list(p.widgets() or [])) for p in b]:
        extra.append("campos")
    if [len(list(p.annots() or [])) for p in a] != [len(list(p.annots() or [])) for p in b]:
        extra.append("anotaciones")
    return "idéntico" if not extra else "píxeles iguales pero cambia: " + ",".join(extra)


def scenarios(data, password):
    out = {}
    doc = open_doc(data, password)
    out["guardar"] = compare(data, doc.save(), password)
    first = doc.save()
    again = open_doc(first)
    out["guardar x2"] = compare(data, again.save(), password)
    again.close()
    n = t.lib.pdfcore_page_count(doc.handle)
    t.call("pdfcore_rotate_page", doc.handle, 0, 90)
    doc.undo()
    out["girar y deshacer"] = compare(data, doc.save(), password)
    if n > 1:
        t.call("pdfcore_move_page", doc.handle, 0, n - 1)
        t.call("pdfcore_move_page", doc.handle, n - 1, 0)
        out["mover y volver"] = compare(data, doc.save(), password)
    saved = doc.save()
    one = (ctypes.c_int * 1)(0)
    added = t.call("pdfcore_insert_pages", doc.handle, saved, len(saved), None, one, 1, 0)
    if added.get("ok"):
        t.call("pdfcore_delete_page", doc.handle, 0)
        out["traer y borrar página"] = compare(data, doc.save(), password)
    doc.close()
    doc = open_doc(data, password)
    runs = doc.page(0)["runs"]
    editable = next((i for i, r in enumerate(runs) if r["editable"] and r["text"].strip()), None)
    if editable is not None:
        original = runs[editable]["text"]
        extra = original.rstrip()[-1]  # a letter the font surely has
        first = doc.replace_ex(editable, original + extra, 0, 1)
        if "error" in first:
            out["(info) añadir una letra y quitarla"] = first["kind"]
        else:
            runs = doc.page(0)["runs"]
            back = next((i for i, r in enumerate(runs) if r["text"] == original + extra), None)
            if back is None:
                out["(info) añadir una letra y quitarla"] = "el tramo se partió en varios"
            else:
                doc.replace_ex(back, original, 0, 1)
                out["(info) añadir una letra y quitarla"] = compare(data, doc.save(), password)
    doc.close()
    return out


if __name__ == "__main__":
    files = [f for f in sorted(glob.glob(str(ROOT / "pdfcore/tests/fixtures/*.pdf"))) if pathlib.Path(f).stem not in SKIP]
    files += sorted(glob.glob("/root/.claude/uploads/*/*.pdf")) + sys.argv[1:]
    summary = {"idéntico": 0, "otro": 0}
    for f in files:
        name = pathlib.Path(f).stem
        data = pathlib.Path(f).read_bytes()
        try:
            result = scenarios(data, PASSWORDS.get(name, ""))
        except Exception as e:  # a check must report, not stop
            print(f"## {name}: FALLO {e!r}")
            summary["otro"] += 1
            continue
        bad = {k: v for k, v in result.items() if v != "idéntico" and not k.startswith("(info)")}
        summary["idéntico" if not bad else "otro"] += 1
        print(f"## {name}: {'todo idéntico' if not bad else bad}")
        for k, v in result.items():
            if k.startswith("(info)") and v != "idéntico":
                print(f"     {k}: {v}")
    print(f"\n{summary['idéntico']} de {len(files)} PDF idénticos en todos los escenarios; {summary['otro']} con diferencias")
