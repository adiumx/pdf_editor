"""Builds the small PDFs the engine's tests read (dev-only, uses PyMuPDF).

The expected text and positions live in the Rust tests next to the numbers
used here; change one, change the other.

    python3 tools/make_fixtures.py
"""
import pathlib
import pymupdf

OUT = pathlib.Path(__file__).resolve().parents[1] / "pdfcore/tests/fixtures"
SANS = "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"
OUT.mkdir(parents=True, exist_ok=True)

# 1. Standard fonts: no /Widths, no embedding, WinAnsi codes.
doc = pymupdf.open()
page = doc.new_page(width=595, height=842)
page.insert_text((72, 100), "Hola mundo", fontname="helv", fontsize=12)
page.insert_text((72, 130), "Negrita áéíóú ñ", fontname="hebo", fontsize=14)
doc.save(OUT / "standard.pdf", garbage=0, deflate=False)

# 2. An embedded TrueType, written as a composite (Type0) font with a ToUnicode.
doc = pymupdf.open()
page = doc.new_page(width=595, height=842)
page.insert_font(fontname="F0", fontfile=SANS)
page.insert_text((72, 200), "Póliza número 12345", fontname="F0", fontsize=11)
try:
    doc.subset_fonts()  # keep only the glyphs used, like a real producer does
except Exception:
    pass
# Compressed on purpose: the engine must read Flate streams, not just plain ones.
doc.save(OUT / "embedded.pdf", garbage=3, deflate=True)

# 3. Text turned a quarter turn.
doc = pymupdf.open()
page = doc.new_page(width=595, height=842)
page.insert_text((300, 400), "Vertical", fontname="helv", fontsize=12, rotate=90)
doc.save(OUT / "rotated.pdf", garbage=0, deflate=False)

# 4. A page that is itself rotated.
doc = pymupdf.open()
page = doc.new_page(width=400, height=600)
page.insert_text((50, 80), "Pagina girada", fontname="helv", fontsize=12)
page.set_rotation(90)
doc.save(OUT / "page_rotated.pdf", garbage=0, deflate=False)
# 5. A page whose content never undoes its own scaling: text added after it
#    must still land where it was asked to.
doc = pymupdf.open()
page = doc.new_page(width=595, height=842)
page.insert_text((72, 100), "Pagina con escala", fontname="helv", fontsize=12)
xref = page.get_contents()[0]
doc.update_stream(xref, b"0.5 0 0 0.5 100 100 cm\nq\n" + doc.xref_stream(xref))
doc.save(OUT / "leaky.pdf", garbage=0, deflate=False)

# 7. One line of three runs (normal + bold + normal), laid out four ways. Real
#    producers do each of these, and a shift that is right for one is wrong for
#    another: a run that follows the pen moves by itself when the one before it
#    changes length, a run with its own position does not.
LONGEST = "Cuarta linea, la mas larga de todas para fijar el margen derecho de la pagina"
R0, R1, R2, NEXT = "El resto de las oficinas de ", "lunes a viernes", " de 9 a 14 horas", "Segunda linea del documento"
w0 = pymupdf.get_text_length(R0, fontname="helv", fontsize=12)
w1 = pymupdf.get_text_length(R1, fontname="hebo", fontsize=12)
x1, x2 = 72 + w0, 72 + w0 + w1


def three_runs(name, build, with_xobject=False):
    doc = pymupdf.open()
    page = doc.new_page(width=595, height=842)
    page.insert_text((10, 10), "x", fontname="helv", fontsize=6)   # registers /helv and /hebo
    page.insert_text((10, 20), "x", fontname="hebo", fontsize=6)
    page.clean_contents()
    fonts = {f[4]: f[0] for f in page.get_fonts(full=True)}
    content_xref = page.get_contents()[0]
    body, extra = build()
    body += (f"BT /helv 12 Tf 72 660 Td {s('Tercera linea del texto')} Tj ET\n"
             f"BT /helv 12 Tf 72 640 Td {s(LONGEST)} Tj ET\n")
    if with_xobject:
        form = doc.get_new_xref()
        doc.update_object(form, f"<< /Type /XObject /Subtype /Form /BBox [0 0 595 842] "
                                f"/Resources << /Font << /hebo {fonts['hebo']} 0 R >> >> >>")
        doc.update_stream(form, extra.encode(), new=True)
        kind, value = doc.xref_get_key(page.xref, "Resources")
        target = int(value.split()[0]) if kind == "xref" else page.xref
        doc.xref_set_key(target, "XObject" if kind == "xref" else "Resources/XObject", f"<< /Fm0 {form} 0 R >>")
    doc.update_stream(content_xref, body.encode())
    doc.save(OUT / f"{name}.pdf", garbage=0, deflate=False)


def s(text): return "(" + text.replace("(", "\\(").replace(")", "\\)") + ")"


three_runs("line_absolute", lambda: (
    f"BT /helv 12 Tf 72 700 Td {s(R0)} Tj ET\n"
    f"BT /hebo 12 Tf {x1:.3f} 700 Td {s(R1)} Tj ET\n"
    f"BT /helv 12 Tf {x2:.3f} 700 Td {s(R2)} Tj ET\n"
    f"BT /helv 12 Tf 72 680 Td {s(NEXT)} Tj ET\n", ""))
three_runs("line_pen", lambda: (
    f"BT 72 700 Td /helv 12 Tf {s(R0)} Tj /hebo 12 Tf {s(R1)} Tj /helv 12 Tf {s(R2)} Tj "
    f"0 -20 Td {s(NEXT)} Tj ET\n", ""))
three_runs("line_relative", lambda: (
    f"BT /helv 12 Tf 72 700 Td {s(R0)} Tj {w0:.3f} 0 Td /hebo 12 Tf {s(R1)} Tj "
    f"{w1:.3f} 0 Td /helv 12 Tf {s(R2)} Tj {-(w0 + w1):.3f} -20 Td {s(NEXT)} Tj ET\n", ""))
three_runs("line_xobject", lambda: (
    f"BT /helv 12 Tf 72 700 Td {s(R0)} Tj ET\n/Fm0 Do\n"
    f"BT /helv 12 Tf 72 680 Td {s(NEXT)} Tj ET\n",
    f"BT /hebo 12 Tf {x1:.3f} 700 Td {s(R1)} Tj ET\n"), with_xobject=True)

# 8. Lines laid out left, centred and right, each in two styles, so the engine can
#    notice (and warn) that pushing what follows would break the alignment.
doc = pymupdf.open()
page = doc.new_page(width=595, height=842)
page.insert_text((10, 10), "x", fontname="helv", fontsize=6)
page.insert_text((10, 20), "x", fontname="hebo", fontsize=6)
page.clean_contents()
left_lines = ["Linea izquierda numero uno del documento", "Otra linea mas corta", LONGEST, "Cierre del texto de la pagina"]
right_edge = 72 + pymupdf.get_text_length(LONGEST, fontname="helv", fontsize=12)
centre = (72 + right_edge) / 2
ca, cb = "Resumen ", "ejecutivo del caso"
wa, wb = pymupdf.get_text_length(ca, fontname="helv", fontsize=12), pymupdf.get_text_length(cb, fontname="hebo", fontsize=12)
cx = centre - (wa + wb) / 2
ra, rb = "Fecha: ", "14 de octubre de 2026"
wra, wrb = pymupdf.get_text_length(ra, fontname="helv", fontsize=12), pymupdf.get_text_length(rb, fontname="hebo", fontsize=12)
rx = right_edge - (wra + wrb)
la, lb = "Nombre: ", "Cesar Ruiz"
wla = pymupdf.get_text_length(la, fontname="helv", fontsize=12)
body = ""
for i, line in enumerate(left_lines):
    body += f"BT /helv 12 Tf 72 {760 - 20 * i} Td {s(line)} Tj ET\n"
body += f"BT /helv 12 Tf {cx:.3f} 600 Td {s(ca)} Tj ET\nBT /hebo 12 Tf {cx + wa:.3f} 600 Td {s(cb)} Tj ET\n"
body += f"BT /helv 12 Tf {rx:.3f} 560 Td {s(ra)} Tj ET\nBT /hebo 12 Tf {rx + wra:.3f} 560 Td {s(rb)} Tj ET\n"
body += f"BT /helv 12 Tf 72 520 Td {s(la)} Tj ET\nBT /hebo 12 Tf {72 + wla:.3f} 520 Td {s(lb)} Tj ET\n"
doc.update_stream(page.get_contents()[0], body.encode())
doc.save(OUT / "aligned.pdf", garbage=0, deflate=False)

# 6. A form with one of each kind of field (reuses the web app's test builder).
import sys
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2]))
from tests.conftest import build_pdf_with_every_kind_of_field
(OUT / "form.pdf").write_bytes(build_pdf_with_every_kind_of_field())
# 9. The form, once with its signature signed, once with a text widget that has
#    no appearance stream (as many real forms have).
from tests.conftest import sign_field
doc = pymupdf.open(stream=(OUT / "form.pdf").read_bytes(), filetype="pdf")
sign_field(doc, [w for w in doc[0].widgets() if w.field_name == "firma"][0].xref)
doc.save(OUT / "signed.pdf", garbage=0, deflate=False)
doc = pymupdf.open(stream=(OUT / "form.pdf").read_bytes(), filetype="pdf")
doc.xref_set_key([w for w in doc[0].widgets() if w.field_name == "nombre"][0].xref, "AP", "null")
doc.save(OUT / "form_no_appearance.pdf", garbage=0, deflate=False)
# 10. A *simple* TrueType font (one byte per code, not a composite one), subset to
#     the glyphs the text uses. Many producers embed fonts this way; letters the
#     text never used are gone from the font program, though the encoding still
#     has a code for them.
doc = pymupdf.open()
page = doc.new_page(width=595, height=842)
page.insert_font(fontname="S0", fontfile=SANS, set_simple=True)
page.insert_text((72, 200), "Poliza numero 12345", fontname="S0", fontsize=11)
page.insert_text((72, 230), "Segunda linea", fontname="S0", fontsize=11)
doc.subset_fonts()
doc.save(OUT / "simple_subset.pdf", garbage=3, deflate=True)
print("fixtures in", OUT)
