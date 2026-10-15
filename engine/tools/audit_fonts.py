"""Font audit (phase 1A): what the engine does with each kind of font.

For every fixture it reads page 1, replaces its first run three ways (plain Latin,
accented, a letter few fonts have) with and without the fallback, and saves with
no edit to compare the drawing against the original. PyMuPDF is only the oracle.

    cargo build -p pdfcore-ffi && python3 tools/audit_fonts.py
"""
import pathlib, sys, unicodedata

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "pdfcore-ffi/tests"))
import pymupdf
import test_abi as t

FIXTURES = [
    ("Type1 estándar sin incrustar (Helvetica, WinAnsi)", "standard"),
    ("Type1 no estándar sin incrustar (Verdana)", "font_nonstd_notembedded"),
    ("TrueType sin incrustar (Calibri-Bold)", "font_truetype_notembedded"),
    ("MMType1 sin incrustar", "font_mmtype1"),
    ("Type1 estándar con MacRoman sin ToUnicode", "font_macroman"),
    ("Type1 estándar sin /Encoding (StandardEncoding)", "font_standardenc"),
    ("Symbol (estándar simbólica)", "font_symbol"),
    ("ZapfDingbats (estándar simbólica)", "font_zapf"),
    ("Type3", "font_type3"),
    ("TrueType simple incrustada, completa", "font_tt_full_simple"),
    ("TrueType simple incrustada, subconjunto", "simple_subset"),
    ("Type0 Identity-H TrueType, completa", "font_tt_full_type0"),
    ("Type0 Identity-H TrueType, subconjunto", "embedded"),
    ("Type0 CIDToGIDMap no trivial", "font_type0_cidtogid"),
    ("Type0 Identity-V (vertical)", "font_type0_identity_v"),
    ("Type0 OpenType-CFF (CIDFontType0)", "cff_type0"),
    ("OpenType-CFF simple", "cff_simple"),
    ("CFF desnudo (Type1C) simple", "cff_bare"),
    ("Type1 incrustada (PFB)", "type1_simple"),
    ("Cirílico y griego (Type0)", "font_cyrillic_greek"),
    ("Árabe, derecha a izquierda (Type0)", "font_rtl_arabic"),
    ("CJK (Type0 OpenType-CFF)", "font_cjk"),
]
TEXTS = {"latino": "Hola mundo 123", "acentos": "Camión ñandú ÁÉÍÓÚ", "rara": "Žluťoučký"}


def norm(text):
    return " ".join(unicodedata.normalize("NFC", text).split())


def oracle_text(data, page=0):
    return norm(pymupdf.open(stream=data, filetype="pdf")[page].get_text())


def outcome(reply):
    return reply["kind"] if "error" in reply else ("ok" + (f"+resp[{reply['fallback_chars']}]" if reply.get("fallback_chars") else ""))


def audit(name):
    d = t.Doc(name)
    original = (t.FIXTURES / f"{name}.pdf").read_bytes()
    runs = d.page()["runs"]
    row = {"runs": len(runs), "font": runs[0]["base_font"] if runs else "-"}
    mine = norm(" ".join(r["text"] for r in runs))
    theirs = oracle_text(original)
    row["lee"] = "igual" if mine == theirs else f"DIFIERE: motor={mine!r} oráculo={theirs!r}"
    # Round trip without edits: same drawing?
    saved = d.save()
    a = pymupdf.open(stream=original, filetype="pdf")[0].get_pixmap(dpi=96)
    b = pymupdf.open(stream=saved, filetype="pdf")[0].get_pixmap(dpi=96)
    diff = sum(1 for x, y in zip(a.samples, b.samples) if abs(x - y) > 8) if (a.width, a.height) == (b.width, b.height) else -1
    row["ida_vuelta"] = "idéntica" if diff == 0 else f"{diff} muestras distintas"
    d.close()
    if not runs:
        row["reemplazo"] = {"(sin tramos)": "-"}
        return row
    row["reemplazo"] = {}
    for label, text in TEXTS.items():
        cells = []
        for fallback in (0, 1):
            doc = t.Doc(name)
            doc.page()  # the C layer names runs by their id in the last reading
            reply = doc.replace_ex(0, text, 0, fallback)
            cell = outcome(reply)
            if "error" not in reply:
                got = oracle_text(doc.save())
                cell += " ✔texto" if text in got else f" ✘extraído={got!r}"
            cells.append(cell)
            doc.close()
        row["reemplazo"][label] = cells
    return row


if __name__ == "__main__":
    for title, name in FIXTURES:
        try:
            row = audit(name)
        except Exception as e:  # an audit must report, not stop
            print(f"## {title} [{name}]\n   FALLO DE LA AUDITORÍA: {e!r}\n")
            continue
        print(f"## {title} [{name}]")
        print(f"   fuente={row['font']}  tramos={row['runs']}  lectura={row['lee']}  ida y vuelta={row['ida_vuelta']}")
        for label, cells in row["reemplazo"].items():
            print(f"   {label:8} sin respaldo: {cells[0]:40} con respaldo: {cells[1] if isinstance(cells, list) else cells}")
        print()
