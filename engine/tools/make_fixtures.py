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

# 6. A form with one of each kind of field (reuses the web app's test builder).
import sys
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2]))
from tests.conftest import build_pdf_with_every_kind_of_field
(OUT / "form.pdf").write_bytes(build_pdf_with_every_kind_of_field())
print("fixtures in", OUT)
