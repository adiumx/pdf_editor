# Motor PDF para la versión Flutter

Motor propio, en Rust, para leer y editar el texto de un PDF **conservando la
fuente original**. Es la base de la versión Android (y después iOS) del editor,
sin servidor: todo corre en el dispositivo.

La versión web/Python que ya existe (`../app`) no se toca; esta rama la tiene
como referencia y como oráculo de pruebas.

## Por qué así

- **Rust + FFI.** Un solo motor compila a Android e iOS y Flutter lo llama por
  FFI. Java/JVM (PDFBox) no corre en iOS.
- **Sin MuPDF.** MuPDF es AGPL: vender una app con él obliga a entregar todo el
  código fuente, o a comprar licencia a Artifex. El motor depende solo de
  librerías permisivas; `python3 tools/check_licenses.py` lo comprueba y debe
  pasar después de cada `cargo add`/`cargo update`.
- **PyMuPDF sólo como oráculo de pruebas** (`tools/`), nunca dentro del motor
  ni de la app: compara lo que lee este motor con una segunda opinión.
- **Dibujar la página no es trabajo del motor.** Android trae `PdfRenderer` e
  iOS `PDFKit`/CoreGraphics, gratis y ya sin problema de licencia. El motor
  entrega *dónde está cada texto y con qué fuente*; la interfaz lo coloca sobre
  la imagen, igual que la versión web coloca sus `.span` sobre el PNG.

## Estado

| Hito | Estado |
|---|---|
| 1. Leer: páginas, tamaño y rotación, tramos de texto con fuente, tamaño y caja | **hecho** |
| 2. Reemplazar un tramo conservando la fuente (código de cada carácter por `ToUnicode` inverso) | siguiente |
| 2b. Carácter que el subconjunto no trae: incrustar una fuente de respaldo (Liberation Sans, OFL) y avisar | |
| 3. Añadir texto nuevo; campos de formulario (AcroForm) con su apariencia | |
| 4. Guardar, y modelo de deshacer | |
| 5. Capa C ABI + plugin Flutter (`flutter_rust_bridge` o `dart:ffi`) | |
| 6. Imágenes, anotaciones, firmas (aviso), OCR en el dispositivo | |

### Lo que el hito 1 ya lee

- Fuentes simples (Type1, TrueType) con WinAnsi y `Differences`; compuestas
  (Type0/Identity-H) con `ToUnicode`; las 14 estándar sin `/Widths` (tablas
  generadas de las métricas de Adobe, `tools/gen_standard_widths.py`).
- Matrices (`cm`, `Tm`, `Td`...), escalado horizontal, interletraje, `TJ`,
  texto dentro de formularios XObject, texto invisible (capa de un escaneo).
- Rotación de la página y de la línea.

### Comprobado contra los PDF reales

`tools/compare_with_pymupdf.py` lo compara con PyMuPDF. Sobre los formularios de
seguros con los que se probó: el texto coincide carácter por carácter en 8 de 9
páginas y las posiciones a menos de 0,1 pt cuando el tramo se parte igual. Las
diferencias conocidas:

- PyMuPDF une un espacio inicial al tramo siguiente; aquí es un tramo aparte.
- MuPDF descarta glifos idénticos sobreimpresos; aquí se leen los dos.

### Huecos conocidos (aún no implementados)

`CIDToGIDMap` distinto de Identity, escritura vertical, fuentes Type3, PDFs
cifrados con contraseña, codificaciones MacRoman/Standard sin `ToUnicode`,
nombres de glifo fuera de una lista corta (la lista completa de Adobe se añadirá
con el hito 2).

## Comandos

```bash
cd engine
cargo test                                   # pruebas del motor
python3 tools/make_fixtures.py               # regenera los PDF de prueba
python3 tools/check_licenses.py              # sólo licencias permisivas
cargo build --example dump
./target/debug/examples/dump archivo.pdf     # todos los tramos, en TSV
python3 tools/compare_with_pymupdf.py archivo.pdf   # contraste con PyMuPDF
```
