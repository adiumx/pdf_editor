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
| 3a. Campos de formulario (AcroForm): leer y rellenar texto, casillas, radios, listas, con su apariencia | **hecho** |
| 3b. Añadir texto nuevo en cualquier punto de la página (para PDF planos, sin campos) | **hecho** |
| 3c. Reajuste de línea: lo que sigue en la línea se mueve al cambiar la longitud de un texto; avisos | **hecho** |
| 3d. Aviso de firma digital; los campos informan si tienen apariencia | **hecho** |
| 4. Guardar, y deshacer/rehacer | **hecho** |
| 5a. Capa C (`pdfcore-ffi`): cabecera, JSON, probada llamándola desde Python | **hecho** |
| 5b. Paquete Dart (`dart/`): `PdfDocument` y `PdfSession` (isolate), 15 pruebas contra la librería real | **hecho** — guía en [FLUTTER.md](FLUTTER.md) |
| 5c. Enlazar para Android/iOS y la app Flutter | **pendiente**: necesita NDK/Xcode/Flutter, que no hay donde se escribió |
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

### Capa C para Flutter (hito 5a)

`pdfcore-ffi` expone el motor como funciones `extern "C"` (`include/pdfcore.h`)
con un manejador opaco y JSON para los datos, que es lo que `dart:ffi` llama
mejor: un solo `jsonDecode` en lugar de una estructura C por tipo. Los tramos de
texto se nombran por su `id` en la última lectura de la página; cualquier
edición, deshacer o rehacer los invalida y hay que volver a leer. Ningún pánico
cruza la frontera (se vuelve un error), los punteros nulos dan `bad_argument`, y
los errores llevan un `kind` estable (`missing_glyphs` con los caracteres,
`rejected`, `stale_run`...).

- **Probado de verdad contra la biblioteca compartida**, llamándola desde Python
  con `ctypes` como la llamará Dart: `cargo build -p pdfcore-ffi && python3 -m
  pytest pdfcore-ffi/tests` (9 pruebas: leer, reemplazar, ids caducados, respaldo,
  añadir, campos, deshacer/rehacer, guardar y reabrir, abrir 50 veces).
- **Compila** (`cargo check`) para `aarch64-linux-android`, `armv7-linux-androideabi`,
  `x86_64-linux-android`, `aarch64-apple-ios` y `aarch64-apple-ios-sim`. Eso
  comprueba que el código es válido para esas plataformas, **no** que se enlace
  ni que corra en un teléfono: para eso hacen falta el NDK, Xcode y un
  dispositivo, que no hay aquí.
- Sin hacer todavía: los enlaces Dart y la app Flutter. No se escriben sin poder
  ejecutarlos; el contrato está en la cabecera y en las pruebas.

### Deshacer y guardar (hito 4)

Cada operación pública (`replace_text*`, `add_text`, `set_field_value*`) es una
**transacción**: si falla se revierte sola y no entra en el historial; si
funciona es un paso que `undo()`/`redo()` deshacen y rehacen entero.

El historial no copia el documento: mientras una transacción está abierta, se
guarda el valor anterior de cada objeto la primera vez que se toca. Una edición
casi siempre *añade* objetos (un flujo de contenido, una apariencia) y cambia
unos pocos diccionarios pequeños (`/Contents` de la página, el `/V` de un
campo), así que un paso ocupa unos cientos de bytes sea cual sea el tamaño del
PDF. El mismo parche que deshace, al aplicarse, devuelve el que rehace.
Deshacer vuelve a quitar también la fuente incrustada que trajo una edición.

- Límite de 200 pasos; una edición nueva tras deshacer olvida lo rehacible.
- `save_to_bytes` poda los objetos sin referencias **sobre una copia**: el
  documento vivo los conserva, porque deshacer puede volver a necesitarlos.
- El historial no se guarda en el archivo: al cerrarlo se pierde. Tras
  `undo`/`redo` hay que volver a leer las páginas.
- Lo medido: 50 ediciones seguidas se deshacen todas y dejan el texto original.
  No se ha medido todavía el consumo de memoria en un PDF grande.

### Texto nuevo (hito 3b)

`Document::add_text(página, &NewText)` escribe texto en cualquier punto, con
las fuentes Liberation incluidas (sans/serif, negrita, cursiva), color, tamaño,
alineación y ajuste de línea a un ancho. `x, y` están en coordenadas de lector
(origen arriba a la izquierda): con `Anchor::TopLeft` son la esquina donde
cayó el dedo; con `Anchor::Baseline` el arranque de la línea base, para
escribir **sobre** una raya dibujada. En páginas giradas el texto sale derecho.

- Se añade como contenido propio al final de la página, envuelto en `q`/`Q`.
  Se cuentan los `q` que la página dejó abiertos y se cierran todos: una página
  que no deshace su propia escala (`cm` suelto) encogía el texto nuevo a la
  mitad (lo encontró una prueba).
- **Una fuente por documento y estilo, entre sesiones.** Al reabrir un archivo,
  cada sesión incrustaba su propia copia con el mismo nombre, que sustituía a la
  anterior en los recursos de la página; la copia nueva sólo conocía las
  anchuras de sus glifos y el texto antiguo salía con las letras separadas como
  si todas midieran un cuadratín. Ahora se reconoce la ya incrustada (por su
  `BaseFont`) y se reutiliza. Se vio al dibujar el resultado real, no en las
  pruebas; ahora hay una que lo cubre y falla sin el arreglo.

### Formularios (hito 3a)

`Document::fields()` lee el árbol de campos: nombre completo (ruta de `/T`),
tipo, valor y banderas heredados del padre, y los *widgets* (rectángulos) con su
página y su caja ya en coordenadas de lector. `set_field_value(nombre, valor)`
rellena texto (una línea, multilínea con salto de palabra, peine, contraseña,
alineación), listas y desplegables (sólo opciones, salvo desplegable editable),
casillas y radios, y respeta máximo de caracteres y solo lectura. Todo se
valida antes de tocar nada y cada rechazo dice por qué.

- **La apariencia se genera.** Muchos formularios (el de seguros con el que se
  probó) traen campos de texto sin `/AP`: un visor que no la construye enseña una
  caja vacía sobre un campo con valor. Se escribe un flujo de apariencia con la
  fuente del propio campo (`/DA`) o, si no dibuja algún carácter, con la de
  respaldo; también se pintan fondo y borde (`/MK`).
- **Solo lectura marcado por error.** Las 28 casillas de ese formulario llevan
  `/Ff 1` (solo lectura), por eso ningún lector que siga el estándar deja
  marcarlas. `set_field_value` las rechaza; `set_field_value_with(.., true)`
  rellena igualmente, para que la interfaz pregunte a la persona antes.
- Un estado de radio llamado `No` es un estado, no la palabra «desmarcado»: se
  compara primero con los nombres exactos (lo encontró una prueba).
- Aún no: dibujar la fila resaltada de una lista, rotación `/MK /R`, firmas,
  cálculos y JavaScript de los campos.

### Reajuste de línea (hito 3c)

Cada cambio de estilo dentro de una línea es un `TextRun` aparte, y nada ata un
tramo al anterior: un productor puede dar a cada uno su propia matriz, dejar que
el cursor siga solo o usar `Td` relativos. Por eso la diferencia de ancho **no se
suma a ciegas** (movería dos veces los que siguen al cursor). Se hace la edición,
se lee la página otra vez y se compara cada tramo con dónde *debería* estar
(donde estaba, más el desplazamiento que le toca por su sitio en la línea); el que
está en otro sitio recibe un `Td` antes de su operación de dibujo. `Td` mueve todo
lo que viene después, que es lo que quiere el resto de la línea y lo que no quiere
la línea siguiente, así que la siguiente se comprueba igual y se devuelve a su
sitio. Sea cual sea la forma en que el productor posicionó el texto, lo que queda
en la página es lo pedido.

- Modos: `Reflow::Following` (por defecto), `Centered`, `RightAligned`, `None`.
- Se mueven los tramos de la misma línea base, a la derecha, con un hueco de hasta
  1,5 em: las celdas de una tabla, más separadas, no se arrastran.
- `Replaced` informa `moved` y `warnings` (`ExceedsPage`, `ExceedsMargin`,
  `FollowerNotMoved` para un tramo en un XObject, `LineMayBeAligned`, `ReflowFailed`).
- Es parte de la transacción: atómico, y `undo`/`redo` lo deshacen con la edición.
- Probado con una línea de tres tramos (normal + negrita + normal) colocada de
  tres maneras (posición propia, cursor continuo, `Td` relativos) más una con el
  seguidor en un XObject; sin el reajuste, 9 de esas pruebas fallan.
- Comprobado en un CV real: al alargar una etiqueta en negrita, el texto normal
  que le sigue se corre lo justo; en un formulario real, las celdas vecinas de una
  tabla no se mueven.
- Límite: no reparte el espacio de un texto justificado; si se pasa del margen,
  avisa.

**Un defecto que se vio al dibujar el resultado real**: en un CV, un tramo en
negrita con fuente TrueType simple (códigos 0..57, sin `/Encoding`) salía con el
cuadrito de «glifo ausente» donde iba una `y`. Un subconjunto solo trae los glifos
que su texto usó, y el motor adivinaba el código de la letra con la codificación
estándar (121), que en esa fuente no existe, y como no podía comprobar el glifo lo
daba por bueno. Ahora una fuente simple incrustada solo ofrece los códigos que
**el `ToUnicode` declara, el programa de fuente confirma, o la página ya dibuja con
esa fuente**; lo demás va a la fuente de respaldo. Hay un fixture y dos pruebas.

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
