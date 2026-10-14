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
# 12. A harder form: the field kinds and options real forms use that the basic one
#     does not (comb cells, auto-sized multiline text, right alignment, list
#     boxes, checkboxes and radios without any appearance).
doc = pymupdf.open()
page = doc.new_page(width=595, height=842)


def widget(kind, name, rect, **kw):
    w = pymupdf.Widget()
    w.field_type, w.field_name, w.rect = kind, name, pymupdf.Rect(*rect)
    for key, value in kw.items():
        setattr(w, key, value)
    page.add_widget(w)
    return w


T, C, L, B, R = (pymupdf.PDF_WIDGET_TYPE_TEXT, pymupdf.PDF_WIDGET_TYPE_COMBOBOX, pymupdf.PDF_WIDGET_TYPE_LISTBOX,
                 pymupdf.PDF_WIDGET_TYPE_CHECKBOX, pymupdf.PDF_WIDGET_TYPE_RADIOBUTTON)
widget(T, "rfc", (72, 100, 272, 124), text_maxlen=13, field_flags=pymupdf.PDF_TX_FIELD_IS_COMB, text_fontsize=12)
widget(T, "notas", (72, 140, 322, 240), field_flags=pymupdf.PDF_TX_FIELD_IS_MULTILINE, text_fontsize=0)
widget(T, "importe", (72, 260, 222, 284), text_fontsize=11, text_format=0)
page.widgets()  # make sure they exist
widget(C, "estado", (72, 300, 222, 324), choice_values=["Jalisco", "Nuevo León", "Yucatán"], text_fontsize=11)
widget(L, "ramos", (72, 340, 222, 420), choice_values=["Vida", "Gastos médicos", "Auto", "Hogar"],
       field_flags=pymupdf.PDF_CH_FIELD_IS_MULTI_SELECT, text_fontsize=11)
widget(B, "acepto", (72, 440, 90, 458))
widget(R, "sexo_m", (72, 470, 90, 488), field_value="Off")
widget(R, "sexo_f", (110, 470, 128, 488), field_value="Off")
for w in page.widgets():
    if w.field_name == "importe":
        w.text_format = 0
        w.update()
page.insert_text((72, 96), "RFC", fontname="helv", fontsize=9)
doc.save(OUT / "form_hard.pdf", garbage=0, deflate=False)
# ... and the same with every checkbox/radio appearance removed.
doc = pymupdf.open(OUT / "form_hard.pdf")
for w in doc[0].widgets():
    if w.field_type in (B, R):
        doc.xref_set_key(w.xref, "AP", "null")
doc.save(OUT / "form_hard_no_ap.pdf", garbage=0, deflate=False)

# 13. Three pages that repeat a few words, for search and replace-all.
doc = pymupdf.open()
for i in range(3):
    page = doc.new_page(width=595, height=842)
    page.insert_text((72, 100), f"Contrato {i + 1} de la aseguradora Prudential", fontname="helv", fontsize=12)
    page.insert_text((72, 130), "Prudential y Prudential Mexico, prudential", fontname="helv", fontsize=12)
    page.insert_text((72, 160), "Polizas de vida: poliza numero 12345", fontname="helv", fontsize=12)
    page.insert_text((72, 190), "Cliente: ", fontname="helv", fontsize=12)
    page.insert_text((72 + pymupdf.get_text_length("Cliente: ", fontname="helv", fontsize=12), 190),
                     "Pérez Núñez", fontname="hebo", fontsize=12)
doc.save(OUT / "multi.pdf", garbage=0, deflate=False)

# 14. Pages that inherit their size, resources and rotation from the page tree's
#     root, the way many producers write them.
doc = pymupdf.open()
for i in range(2):
    page = doc.new_page(width=400, height=300)
    page.insert_text((50, 100), f"Hereda {i + 1}", fontname="helv", fontsize=14)
root = doc.pdf_catalog()
pages_xref = int(doc.xref_get_key(root, "Pages")[1].split()[0])
for page in doc:
    for key in ("MediaBox", "Resources"):
        kind, value = doc.xref_get_key(page.xref, key)
        if kind != "null":
            doc.xref_set_key(pages_xref, key, value)
            doc.xref_set_key(page.xref, key, "null")
doc.xref_set_key(pages_xref, "Rotate", "90")
for page in doc:
    doc.xref_set_key(page.xref, "Rotate", "null")
doc.save(OUT / "inherited.pdf", garbage=0, deflate=False)

# 15. A scan turned upright by the page's /Rotate: the text runs sideways in the
#     file and straight as a reader shows it.
doc = pymupdf.open()
page = doc.new_page(width=595, height=842)
page.insert_text((100, 700), "Linea girada uno", fontname="helv", fontsize=12, rotate=90)
page.insert_text((130, 700), "Linea girada dos", fontname="helv", fontsize=12, rotate=90)
page.set_rotation(270)
doc.save(OUT / "turned_upright.pdf", garbage=0, deflate=False)

# 16. Pictures to place: left half red, right half blue (so orientation and
#     placement can be told apart), the same with an EXIF orientation of 6, and a
#     PNG whose right half is transparent.
import struct


def halves(w, h, left, right, alpha=None):
    row = bytes(left) * (w // 2) + bytes(right) * (w - w // 2)
    return row * h


rgb = pymupdf.Pixmap(pymupdf.csRGB, 80, 40, halves(80, 40, (255, 0, 0), (0, 0, 255)), False)
jpeg = rgb.tobytes("jpeg", jpg_quality=95)
(OUT / "img_rgb.jpg").write_bytes(jpeg)


def with_exif_orientation(data, orientation):
    tiff = b"MM\x00\x2a\x00\x00\x00\x08" + struct.pack(">H", 1) + struct.pack(">HHIHH", 0x0112, 3, 1, orientation, 0) + b"\x00\x00\x00\x00"
    body = b"Exif\x00\x00" + tiff
    segment = b"\xff\xe1" + struct.pack(">H", len(body) + 2) + body
    return data[:2] + segment + data[2:]


for n in (3, 6, 8):
    (OUT / f"img_exif{n}.jpg").write_bytes(with_exif_orientation(jpeg, n))
import zlib


def png_rgba(w, h, pixels):
    """A PNG with an alpha channel (PyMuPDF has no way to write one)."""
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    raw = b"".join(b"\x00" + pixels[y * w * 4:(y + 1) * w * 4] for y in range(h))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


cells = b"".join((bytes((0, 160, 0, 255)) if x < 20 else bytes((0, 0, 0, 0))) for y in range(40) for x in range(40))
(OUT / "img_alpha.png").write_bytes(png_rgba(40, 40, cells))

# 17. Embedded fonts that are not TrueType: OpenType with CFF outlines (composite
#     and simple), a bare CFF table (`Type1C`), and Type 1. Loma is a Thai font with
#     Latin letters but no Ž or €; the composite one also gets a ToUnicode that lies,
#     claiming an empty glyph (en space) draws Ž.
import struct as _struct
LOMA = "/usr/share/fonts/opentype/tlwg/Loma.otf"
CHARTER = "/usr/share/fonts/X11/Type1/c0648bt_.pfb"


def font_page(path, simple, text="Hola ñandú"):
    doc = pymupdf.open()
    page = doc.new_page(width=595, height=842)
    page.insert_font(fontname="F0", fontfile=path, set_simple=simple)
    page.insert_text((72, 200), text, fontname="F0", fontsize=14)
    return doc, doc[0].get_fonts(full=True)[0][0]


doc, font_xref = font_page(LOMA, False)
tounicode = int(doc.xref_get_key(font_xref, "ToUnicode")[1].split()[0])
cmap = doc.xref_stream(tounicode).decode().replace("endcmap", "1 beginbfchar\n<0124> <017D>\nendbfchar\nendcmap", 1)
doc.update_stream(tounicode, cmap.encode())
doc.save(OUT / "cff_type0.pdf", garbage=0, deflate=False)

doc, font_xref = font_page(LOMA, True)
doc.save(OUT / "cff_simple.pdf", garbage=0, deflate=False)

# The same font with its bare CFF table (no sfnt wrapper), as `Type1C`.
data = open(LOMA, "rb").read()
count = _struct.unpack(">H", data[4:6])[0]
cff = next(data[o:o + n] for tag, _, o, n in (_struct.unpack(">4sIII", data[12 + 16 * i:28 + 16 * i]) for i in range(count)) if tag == b"CFF ")
doc, font_xref = font_page(LOMA, True)
descriptor = int(doc.xref_get_key(font_xref, "FontDescriptor")[1].split()[0])
program = int(doc.xref_get_key(descriptor, "FontFile3")[1].split()[0])
doc.update_stream(program, cff)
doc.xref_set_key(program, "Subtype", "/Type1C")
doc.save(OUT / "cff_bare.pdf", garbage=0, deflate=False)

doc, _ = font_page(CHARTER, True)
doc.save(OUT / "type1_simple.pdf", garbage=0, deflate=False)
charter = pymupdf.Font(fontfile=CHARTER)
loma = pymupdf.Font(fontfile=LOMA)
(OUT / "fonts_missing.txt").write_text(
    "charter " + "".join(c for c in "ŽžŠšÿ€Ðð" if not charter.has_glyph(ord(c))) + "\n"
    "loma " + "".join(c for c in "ŽžŠšÿ€Ðð" if not loma.has_glyph(ord(c))) + "\n")

# 11. Encrypted copies of the simple text page. Most producers write /Encrypt as
#     an indirect object, which MuPDF does not; `indirect_encrypt` rewrites it
#     that way with an incremental update. One file keeps MuPDF's inline form.
import re


def indirect_encrypt(data):
    m = list(re.finditer(rb"trailer\s*<<(.*)>>\s*startxref\s*(\d+)\s*%%EOF", data, re.S))[-1]
    body, prev = m.group(1), int(m.group(2))
    start = body.index(b"/Encrypt") + len(b"/Encrypt")
    while body[start:start + 1] in b" \n\r":
        start += 1
    depth, at = 0, start
    while True:
        if body[at:at + 2] == b"<<":
            depth += 1
            at += 2
        elif body[at:at + 1] == b"<":  # a hex string
            at = body.index(b">", at) + 1
        elif body[at:at + 2] == b">>":
            depth -= 1
            at += 2
            if depth == 0:
                break
        else:
            at += 1
    encrypt = body[start:at]
    size = int(re.search(rb"/Size (\d+)", body).group(1))
    root = re.search(rb"/Root \d+ \d+ R", body).group(0)
    ident = re.search(rb"/ID\s*\[.*?\]", body, re.S).group(0)
    out = data + b"\n"
    offset = len(out)
    out += b"%d 0 obj\n" % size + encrypt + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n%d 1\n%010d 00000 n \n" % (size, offset)
    out += b"trailer\n<</Size %d%s%s/Encrypt %d 0 R/Prev %d>>\nstartxref\n%d\n%%%%EOF\n" % (
        size + 1, root, ident, size, prev, xref)
    return out


for name, user, algo in [
    ("encrypted_owner", "", pymupdf.PDF_ENCRYPT_AES_256),
    ("encrypted_rc4", "", pymupdf.PDF_ENCRYPT_RC4_128),
    ("encrypted_user", "secreto", pymupdf.PDF_ENCRYPT_AES_256),
]:
    doc = pymupdf.open(stream=(OUT / "standard.pdf").read_bytes(), filetype="pdf")
    raw = doc.tobytes(encryption=algo, owner_pw="dueno", user_pw=user,
                      permissions=pymupdf.PDF_PERM_PRINT, garbage=0, deflate=False)
    (OUT / f"{name}.pdf").write_bytes(indirect_encrypt(raw))
    if name == "encrypted_owner":
        (OUT / "encrypted_inline.pdf").write_bytes(raw)
print("fixtures in", OUT)
