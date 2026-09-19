#!/usr/bin/env python3
"""Build a sample PDF with embedded fonts, for trying the editor out.

    python tools/make_sample_pdf.py ejemplo.pdf
"""

from __future__ import annotations

import sys

import pymupdf

FONTS = {
    "serif": "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
    "serif-bold": "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
    "serif-italic": "/usr/share/fonts/truetype/liberation/LiberationSerif-Italic.ttf",
    "sans": "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
}


def build(path: str) -> None:
    doc = pymupdf.open()

    page = doc.new_page()
    for name, file in FONTS.items():
        page.insert_font(fontname=name, fontfile=file)

    page.insert_text((72, 96), "Informe trimestral", fontname="serif-bold", fontsize=22)
    page.insert_text((72, 118), "Área de operaciones · marzo", fontname="serif-italic", fontsize=11,
                     color=(0.35, 0.35, 0.4))

    body = [
        "Este párrafo existe para comprobar que el editor conserva la fuente",
        "embebida del documento al reescribir una línea. Cambia cualquier",
        "palabra y compárala con las líneas de alrededor: deben seguir",
        "pareciendo el mismo texto.",
    ]
    y = 156
    for line in body:
        page.insert_text((72, y), line, fontname="serif", fontsize=12)
        y += 17

    page.insert_text((72, 250), "Una línea en negrita", fontname="serif-bold", fontsize=13)
    page.insert_text((72, 272), "Otra en cursiva y en color", fontname="serif-italic", fontsize=13,
                     color=(0.1, 0.25, 0.7))
    page.insert_text((72, 300), "Sans-serif para contrastar", fontname="sans", fontsize=13)

    # A right-aligned block, so alignment detection has something to find.
    for index, line in enumerate(["Total: 1.240,00 €", "IVA: 260,40 €", "A pagar: 1.500,40 €"]):
        width = pymupdf.Font(fontfile=FONTS["sans"]).text_length(line, 12)
        page.insert_text((523 - width, 360 + index * 16), line, fontname="sans", fontsize=12)

    second = doc.new_page()
    second.insert_font(fontname="serif", fontfile=FONTS["serif"])
    second.insert_text((72, 96), "Segunda página", fontname="serif", fontsize=18)
    second.insert_text((72, 130), "Para probar el reordenado y el borrado de páginas.",
                       fontname="serif", fontsize=12)

    doc.save(path, garbage=4, deflate=True)
    doc.close()
    print(f"Escrito {path}")


if __name__ == "__main__":
    build(sys.argv[1] if len(sys.argv) > 1 else "ejemplo.pdf")
