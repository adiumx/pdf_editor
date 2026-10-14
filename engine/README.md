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
| 2. Reemplazar un tramo conservando la fuente, con la que ya trae el PDF | **hecho** |
| 2b. Carácter que el subconjunto no trae: se dibuja con una fuente de respaldo incluida (Liberation Sans/Serif, OFL) y se avisa cuáles | **hecho** |
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

### Lo que el hito 2 hace, y cómo se comprobó

`Document::replace_text` cambia el operando de la única operación de dibujo que
mostró el tramo, **en su sitio**: el número de operaciones no cambia, así que
las lecturas siguen valiendo y se pueden encadenar varias ediciones. El
carácter se busca en el `ToUnicode` al revés (o en la codificación, en fuentes
simples) y se escribe con el código de la propia fuente.

Un `ToUnicode` no garantiza que el glifo exista: un subconjunto conserva el
hueco y el mapa de un glifo que ya no dibuja nada, y escribir con ese código
daría texto en blanco sin aviso. Por eso se mira el programa de fuente
(`FontFile2`, con `ttf-parser`) y un código sin contorno se rechaza. Lo
descubrió una prueba que decía que sí existían `X` y `Q`; ahora fallan limpio.

Comprobado en un formulario real de seguros: se cambió una línea en Arial
(Type0/Identity-H, subconjunto), MuPDF abre el resultado, la dibuja con la
misma fuente en el mismo sitio, y es la única diferencia de texto de la página.

### Fuente de respaldo (hito 2b)

`replace_text` es estricto: si la fuente del PDF no trae un carácter, falla
con `MissingGlyphs` y no cambia nada. `replace_text_with_fallback` dibuja sólo
esos caracteres con una fuente incluida del estilo más cercano (serif o sans,
negrita, cursiva) y el resto con la original. Liberation es métricamente
compatible con Arial/Helvetica y Times, así que la letra cambiada ocupa lo que
ocuparía la original. El color y el tamaño no cambian: son estado gráfico, no de
la fuente.

- Devuelve en `Replaced` qué caracteres usaron respaldo (`fallback_chars`) para
  poder decírselo a la persona, y `reread: true` si hubo que insertar cambios de
  fuente: la página se vuelve a leer antes de la siguiente edición.
- La fuente se incrusta **una vez por documento y estilo**, comprimida (~200 KB);
  sólo crece su tabla de anchuras y el mapa de texto, así que el texto se puede
  buscar y copiar.
- Si ni el respaldo trae el carácter (por ejemplo CJK) falla igual con
  `MissingGlyphs` y sin cambios. Pendiente: Liberation Mono y subconjuntar la
  fuente incrustada para que pese menos.
- Licencia: las fuentes son SIL OFL 1.1 (`pdfcore/fonts/LICENSE-Liberation-OFL.txt`),
  que permite incluirlas e incrustarlas en un producto de pago.

### Huecos conocidos (aún no implementados)

verificación de glifos en fuentes CFF/Type1 (sólo se comprueban TrueType), editar texto dentro de formularios XObject, un cambio de longitud no desplaza lo que sigue en la línea, escritura vertical, fuentes Type3, PDFs
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
