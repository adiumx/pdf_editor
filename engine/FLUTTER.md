# Usar el motor en Flutter

Guía para llevar `pdfcore` a una app Flutter (Android ahora, iOS después).

## Qué está probado y qué no

| Pieza | Estado |
|---|---|
| Motor Rust (`pdfcore`) | Probado: más de 250 pruebas, fuzz con ~25 mil archivos dañados, y contra PDF reales de seguros |
| Capa C (`pdfcore-ffi`) | Probada llamándola desde Python (`ctypes`) y desde Dart |
| Paquete Dart (`engine/dart`) | **Probado**: 35 pruebas con `dart test` contra la librería real, en Linux. Una versión anterior ya se integró y probó en una app Flutter en Windows y en un emulador Android (lectura, edición, formularios, guardado) |
| Compilar para Android/iOS | `cargo check` pasa; **el enlazado y la ejecución en un teléfono no se han probado** (no hay NDK, Xcode ni Flutter donde se escribió) |
| La interfaz Flutter de abajo | **Boceto sin compilar**: muestra cómo se usa la API, no es una app terminada |

Los pasos 1 y 2 son los que hay que ejecutar en tu máquina; si algo falla ahí, es
lo primero que hay que mirar.

## Cómo encaja

```
 PDF ──► pdfcore (Rust) ──► pdfcore-ffi (C) ──► paquete Dart `pdfcore` ──► tu app Flutter
          lee, edita,         JSON + punteros     PdfSession (isolate)
          guarda              opacos
```

El motor **no dibuja la página**: dice *dónde está cada texto y con qué fuente*.
La imagen la pone un visor del sistema (Android `PdfRenderer`, iOS PDFKit; el
paquete `pdfx` los envuelve — comprueba su licencia antes de vender) y tu app
coloca los cuadros editables encima, escalando `display` por el zoom.

## 1. Compilar la librería nativa

Necesitas Rust, el NDK de Android (desde Android Studio → SDK Manager → NDK) y
[`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk):

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<versión>

cd engine
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
  -o ../mi_app/android/app/src/main/jniLibs \
  build --release -p pdfcore-ffi
```

Eso deja `libpdfcore_ffi.so` en `jniLibs/<arquitectura>/`, donde Android la busca
sola. `arm64-v8a` cubre las tablets Galaxy Tab S7 FE/S9/S10; `armeabi-v7a` los
equipos viejos de 32 bits; `x86_64` el emulador.

El perfil `release` ya está ajustado para tamaño (`opt-level = "s"`, LTO, strip).
Las fuentes incluidas (Liberation Sans/Serif/Mono, Carlito, Caladea, Open Sans, Noto Serif: unos 10 MB en el binario) se
**subconjuntan al incrustarlas**: un documento solo lleva los contornos de las letras que usó (unos
kilobytes por fuente y estilo, no ~200 KB). Si el binario pesa demasiado, se pueden quitar
familias enteras de `pdfcore/src/fallback.rs`.

**iOS** (en un Mac con Xcode):

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build --release -p pdfcore-ffi --target aarch64-apple-ios
cargo build --release -p pdfcore-ffi --target aarch64-apple-ios-sim
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libpdfcore_ffi.a -headers pdfcore-ffi/include \
  -library target/aarch64-apple-ios-sim/release/libpdfcore_ffi.a -headers pdfcore-ffi/include \
  -output PdfCore.xcframework
```

Arrastra el `.xcframework` a Xcode (Runner → Frameworks) y añade
`-force_load <ruta>/libpdfcore_ffi.a` en *Other Linker Flags*: el paquete Dart
busca los símbolos dentro de la propia app (`DynamicLibrary.process()`), y sin
`force_load` el enlazador los descarta por no verlos usados.

## 2. Añadir el paquete Dart

En el `pubspec.yaml` de tu app:

```yaml
dependencies:
  pdfcore:
    path: ../engine/dart        # o la ruta donde lo copies
```

Para comprobar el paquete sin Flutter (en tu máquina de escritorio):

```bash
cd engine && cargo build -p pdfcore-ffi
cd dart && dart test            # 15 pruebas contra la librería de verdad
```

## 3. Usarlo

Siempre con `PdfSession` en una app: corre el motor en un isolate propio y cada
llamada es un `Future`, así abrir un PDF grande no congela la pantalla.
(`PdfDocument` es lo mismo en síncrono, útil en pruebas.)

```dart
import 'package:pdfcore/pdfcore.dart';

final bytes = await File(ruta).readAsBytes();
final session = await PdfSession.open(bytes);      // lanza PdfException si no es un PDF

final page = await session.page(0);                // tamaño y textos de la página
for (final run in page.runs) {
  // run.text, run.baseFont, run.size
  // run.display = [x0, y0, x1, y1] en puntos, origen arriba a la izquierda
}

// Editar un texto conservando su fuente:
final done = await session.replaceText(page, page.runs[0], 'Texto nuevo');

// Escribir en un punto cualquiera (PDF plano, sin campos):
await session.addText(0, NewText('Nombre', 72, 300, size: 12, bold: true));

// Rellenar un formulario:
final fields = await session.fields();             // nombre, tipo, valor, opciones, widgets
await session.setField('p1_fecha', '14/10/2026');
await session.setField('chk_cambio_contratante', 'true');

await session.undo();  await session.redo();
final saved = await session.save();                // Uint8List: escríbelo a un archivo
await session.close();
```

### Reglas que hay que respetar

1. **Los `run.id` caducan.** Cualquier edición, `undo` o `redo` los invalida: vuelve
   a llamar a `session.page(n)` antes de la siguiente edición. Si no, `PdfException`
   con `isStaleRun`.
2. **Letras que la fuente no trae.** `replaceText` falla con `isMissingGlyphs` y
   `e.chars` dice cuáles. Pregunta a la persona («la fuente del documento no
   tiene Ñ, ¿uso una parecida?») y reintenta con `allowFallback: true`. `done.fallbackChars`
   dice qué letras usaron respaldo.
3. **Campos de solo lectura.** `setField` devuelve `void` y **avisa de un rechazo
   lanzando `PdfException`** (nunca devuelve un resultado que leer). Muchos formularios
   marcan por error como solo lectura campos que sí se deben rellenar (las 28 casillas
   del formato de Prudential). Pregunta a la persona y reintenta con
   `ignoreReadOnly: true`:

   ```dart
   try {
     await session.setField('chk_cambio_contratante', 'true');
   } on PdfException catch (e) {
     // e.kind: 'rejected' (solo lectura, demasiado largo, opción inválida),
     //         'no_such_field', 'unsupported' (firma, botón)...  e.message dice por qué
     if (e.kind == 'rejected' && await confirmarConLaPersona(e.message)) {
       await session.setField('chk_cambio_contratante', 'true', ignoreReadOnly: true);
     }
   }
   ```
4. **Texto largo en un campo.** El motor rechaza lo que pase de `maxLen` o meta un
   salto de línea en un campo de una línea; el mensaje dice por qué.
5. **Cada operación es atómica**: si falla, nada cambió y no hay nada que deshacer.
6. **Guardar y volver a dibujar.** El visor del sistema dibuja un archivo, no el
   estado interno: tras cada edición, `session.save()` y vuelve a abrir esa página
   en el visor. Con PDF grandes conviene redibujar solo la página editada.
7. **Firmas digitales.** Editar invalida una firma. Antes de la primera edición,
   `final firmas = await session.signatures();` y, si no está vacía, avisa a la
   persona (`firmas.first.signer` dice quién firmó, si la firma lo indica).
   `fields()` también marca `signed` y `signer` en cada campo de firma.
8. **Los espacios finales importan.** Los tramos de un PDF suelen llevar el espacio
   que los separa del siguiente (`"Desarrollo Full Stack: "`). Si el texto nuevo no
   lo conserva, queda «Cloud:Java» sin hueco. Al abrir un tramo para editar,
   conserva sus espacios iniciales y finales.

## 4. Cambiar la longitud de un texto

Cada cambio de estilo dentro de una línea es un `TextRun` aparte (por ejemplo una
etiqueta en negrita seguida de texto normal). Si el texto nuevo mide distinto, lo
que sigue en la misma línea **se desplaza solo**: así no se empalma ni queda un hueco.

```dart
final r = await session.replaceText(page, run, nuevo);        // Reflow.following por defecto
// r.moved     → cuántos tramos se movieron para hacer sitio
// r.warnings  → cosas que la persona debería saber
```

`reflow:` elige qué se mantiene fijo: `Reflow.following` (lo que sigue se mueve; para
texto alineado a la izquierda), `Reflow.centered` (la línea conserva su centro),
`Reflow.rightAligned` (conserva el borde derecho) y `Reflow.none`.

Qué se mueve y qué no: solo los tramos de **la misma línea base**, a la derecha,
con un hueco de hasta 1,5 veces el tamaño de letra (unos pocos espacios). Las celdas
de una tabla, más separadas, no se arrastran; las otras líneas, tampoco.

`r.warnings` (cada uno con `kind`):

| `kind` | Qué significa |
|---|---|
| `exceeds_page` (`by`) | la línea ahora se sale de la página por `by` puntos |
| `exceeds_margin` (`by`) | pasa del punto donde terminan las demás líneas |
| `follower_not_moved` (`text`) | un tramo que debía moverse está en un formulario XObject (se lee pero no se edita) y se quedó donde estaba |
| `line_may_be_aligned` (`alignment`) | la línea parece centrada o a la derecha: quizá convenga `Reflow.centered` / `rightAligned` |
| `reflow_failed` | no se pudo recolocar; el texto quedó como se escribió |

El reajuste es parte de la misma operación: es atómico, `undo()`/`redo()` lo deshacen
junto con la edición, y los `run.id` caducan como siempre (`r.reread` dice si hubo
que insertar operaciones).

## 5. Más operaciones del motor

Todas están en `PdfDocument` y `PdfSession`, devuelven errores como `PdfException`
(con `kind`) y las que cambian el documento se deshacen con `undo()` en un solo paso.

```dart
// PDF con contraseña: 'password_required' si hace falta, 'wrong_password' si está mal.
final session = await PdfSession.open(bytes, password: clave);

// Formularios: orden de relleno para «siguiente campo»; listas con varias filas.
final campos = (await session.fields())..sort((a, b) => a.order.compareTo(b.order));
await session.setField('ramos', 'Vida\nAuto');          // una opción por línea

// Texto como la persona lo lee, y buscarlo.
final lineas = await session.lines(0);                  // líneas → palabras con su caja
final hallazgos = await session.find('prudential');     // sin distinguir mayúsculas ni acentos
final r = await session.replaceAll('Prudential', 'Metlife');   // un solo deshacer
// r.replaced, r.skipped (cruzan dos cadenas de texto y no se tocaron)

// Páginas.
await session.rotatePage(0, 90);
await session.movePage(0, 2);
await session.deletePage(1);
await session.insertPages(otroPdf, at: 1, pages: [0, 3]);     // trae sus campos
final parte = await session.extractPages([2, 0]);             // PDF nuevo

// Imágenes (JPEG se incrusta tal cual; PNG conserva su transparencia).
await session.addImage(0, fotoBytes, x: 72, y: 600, width: 120);

// Campos: mover/redimensionar (la cuadrícula decide la caja; el motor la mantiene en la página)
final dondeQuedo = await session.moveField('nombre', [100, 300, 260, 322]);
await session.arrangeFields(
  [FieldRefs('rfc'), FieldRefs('notas'), FieldRefs('importe')], Arrange.alignLeft); // un solo deshacer
// Ojo: arrangeFields alinea los campos entre sí, no los ajusta a la cuadrícula. Para ajustar a la
// cuadrícula, calcula la caja de cada campo y llama a moveField por campo: cada llamada es un
// paso de deshacer aparte.

// Elementos como en Canva: textos, imágenes y campos sueltos. Un solo deshacer cada operación.
final elementos = await session.items(0);               // texto (un párrafo = uno), imágenes, campos
final elegidos = elementos.where((e) => seleccion.contains(e.id)).toList();
await session.arrangeItems(0, elegidos, Arrange.alignLeft);      // o centerHorizontally, alignRight, ...
await session.arrangeItems(0, elegidos, Arrange.distributeVertically);   // huecos iguales (3 o más)
await session.spaceLines(0, elegidos, leading: 18);     // interlineado uniforme (null = el promedio)
await session.moveItems(0, [ItemMove(elegidos.first, 10, -4)]);
// Alinear el texto DENTRO de un párrafo o de un campo (botones de párrafo de Word):
await session.alignParagraph(page, run, TextAlign.center);
await session.setFieldsAlignment(['nombre', 'rfc'], TextAlign.right);

// Párrafos: muestra todo el texto en un cuadro y devuélvelo completo.
final p = await session.paragraphOf(page, run);         // null si es una línea suelta
if (p != null) {
  final res = await session.replaceParagraph(page, run, textoEditado);
  // res.added / res.removed líneas; res.warnings: overlaps_text, below_page
}
```

Qué hay que tener presente:

- `replaceParagraph` **no empuja** lo que hay debajo: si el párrafo crece sobre otro
  texto, avisa con `overlaps_text` y la persona decide (puede deshacer).
- Un PDF cifrado se guarda sin protección.
- `insertPages` no hace únicos los nombres de campo: dos formularios con un campo
  `nombre` comparten el valor al rellenarlo.
- Tras cualquier operación de las anteriores, los `run.id` caducan: vuelve a leer la página.
- Ajustar campos a una cuadrícula con `moveField` deja un paso de deshacer por campo; `arrangeFields` sí es uno solo, pero no usa la cuadrícula.

### Párrafos con varios estilos: un solo recuadro

Un párrafo que mezcla negritas, tamaños o colores NO se parte en recuadros: es un
`PdfBlock` con `text` completo y `spans` (`start`/`end` en caracteres, `bold`, `italic`,
`size`, `color`). Muestra un solo cuadro de edición y, al terminar, llama a
`doc.replaceBlock(page, block, nuevoTexto)`: cada letra que se queda conserva su estilo,
lo que se escribe toma el de la letra anterior, las líneas que no cambian no se tocan
(`kept`) y el justificado y la sangría se mantienen. Es un paso de deshacer.

- JSON: `pdfcore_blocks` → `{"blocks":[{id,text,display,lines,spans,justified,first_line_indent,editable,reason}]}`;
  `pdfcore_replace_block` → `{ok,lines,kept,rewritten,added,removed,fallback_chars,warnings}`.
- Error `stale_block` (`PdfException.isStaleBlock`): los ids caducan con cualquier edición; vuelve a llamar a `blocks`.
- Las listas y columnas de etiquetas (casillas, "Nombre / Teléfono") no se unen en un párrafo.
- No verificado: PDF cuyo párrafo está dibujado con una línea por operador en orden distinto al visual.

### Estilo por tramo, reemplazar un rango, empujar lo de abajo

Sobre un `PdfBlock` (siempre uno por párrafo o por línea suelta):

- `setBlockStyle(page, block, start, end, bold:, italic:, size:, color:)`: cambia el estilo de las letras
  `[start, end)`. La negrita o cursiva usa la fuente del propio documento si la página la trae
  (`Arial` → `Arial-BoldMT`); si no, una incluida, que exige `allowFallback` (si no: `rejected`) y sale
  en `substitutes`. Tamaño y color se aplican a las letras elegidas y el párrafo se recoloca.
- `replaceRange(page, block, start, end, text)`: reemplaza solo ese rango; el texto nuevo toma el
  estilo de la primera letra que reemplaza (como una selección en un procesador de texto).
- `replaceBlock(..., justify: true|false, pushDown: true)`: `justify` fuerza justificado o bandera;
  `pushDown` baja lo que hay debajo (texto, imágenes, campos) tanto como ocupan las líneas añadidas;
  lo que no cabría en la página se queda y avisa con `below_page`. Sin `pushDown`, si el párrafo
  crece sobre otro texto, `overlaps_text`.
- `blockChars(page, block)`: caja de cada letra para colocar el cursor o una selección.

Todo es un paso de deshacer, y los ids caducan con cualquier edición. Si un texto no es un párrafo
(una sola línea) funciona igual con un bloque de una línea.

### Lotes, borrar, tocar, redimensionar, apariencia de formularios

- `batch([BatchOp...])`: varias ediciones en UN paso de deshacer; si una falla no se aplica ninguna.
  Cada operación apunta a ids de la última lectura **tal como estaban al empezar el lote** (el motor los
  vuelve a encontrar por texto y posición), y deben apuntar a cosas distintas. En Rust, `Document::batch`
  hace lo mismo con cualquier secuencia de llamadas.
- `deleteItems(page, items)`: el texto deja de dibujarse, la imagen se cancela, el campo sale de la página y
  del formulario. Un paso; deshacer devuelve el archivo idéntico.
- `hitTest(page, x, y)`: qué hay bajo un punto (campo antes que texto antes que imagen). Para texto devuelve
  el bloque, la letra y la posición del cursor (`caret`, 0..largo). Vuelve a leer ítems y bloques, así que
  los ids de la respuesta son los vigentes.
- `resizeItem(page, item, box)`: la imagen se escala, el campo toma la caja (y reconstruye su apariencia), el
  texto se recoloca dentro del nuevo ancho (la altura la da el texto).
- `refreshAppearances(onlyMissing:)`: reconstruye la apariencia (`/AP`) de los campos desde su valor, para
  visores que no la generan; marca/casillas reciben los estados que les falten.

### Anotaciones, aplanar, censurar, informe de OCR y guardado incremental

- **Anotaciones**: `annotations(page)` las lista (`kind`: `highlight`, `underline`, `strike_out`, `squiggly`, `note`,
  `free_text`, `ink`, `square`, `circle`, `line`, `link`, ...). `addAnnotation(page, NewAnnotation.highlight(boxes) |
  .note(x, y, texto) | .freeText(rect, texto) | .ink(trazos) | .rectangle(...) | .ellipse(...) | .line(...))` crea
  la anotación **con su apariencia**, así que cualquier visor la dibuja (MuPDF las lee como anotaciones del
  tipo correcto). `setAnnotationContents`, `moveAnnotation` y `deleteAnnotation`. Cada una es un paso de
  deshacer. El texto libre solo admite lo que Helvetica/WinAnsi dibuja (si no, `missing_glyphs`).
- **Aplanar** (`flatten(pages:, annotations:, forms:)`): dibuja anotaciones y campos en la propia página y los quita
  (la copia para enviar o imprimir). Las firmas no se aplanan (`skipped`); un documento firmado deja de ser
  válido. Los enlaces se conservan.
- **Censurar** (`redact(page, boxes, removeImages:)`): las letras cuyo centro cae en una zona **se quitan del
  archivo** (no solo se tapan); el resto del texto no se mueve (un hueco de `TJ` exacto). Anotaciones y campos bajo la
  zona se quitan. Las imágenes que tocan la zona se tapan pero **sus píxeles siguen en el archivo** (`imagesKept`)
  salvo `removeImages: true`, que las quita enteras. Texto dentro de un XObject de formulario, o dibujado con
  `'`/`"`, rechaza toda la censura (`unsupported`) para no dejar nada legible por error. No toca metadatos ni
  revisiones anteriores de un archivo guardado de forma incremental.
- **Informe de OCR** (`ocrReport()`): por página `native`, `scanned_no_text` (ofrece OCR: `needsOcr`),
  `scanned_with_text_layer`, `image_only` o `blank`. El motor no reconoce caracteres.
- **Guardado incremental** (`saveIncremental()`): el archivo original intacto al principio y los cambios añadidos
  al final; una firma conserva su rango de bytes. También para archivos con contraseña.

### Gráficos vectoriales (`graphics`)

`doc.graphics(page)` / `session.graphics(page)` devuelve lo que la página **pinta** con trazos: rectángulos
(casillas, celdas, marcos), líneas y otros caminos, en orden de pintado y con coordenadas como se ve la página
(origen arriba a la izquierda, rotación y `cm` aplicados, formularios XObject incluidos). Cada `PdfGraphic` trae
`kind` (`rect`, `line`, `path`), `display`, `filled`/`stroked`, `fill`/`stroke` (RGB si el color es llano),
`lineWidth`, `xobject` y, en las líneas, `ends`. Un recorte (`W n`) no es un gráfico. Las letras convertidas a
curvas también salen: filtra por tamaño (por ejemplo `isRect` con ~9×9 pt para casillas). Ejemplo: en
`CAMBIO_DE_CONTRATANTE.pdf` la página 0 trae las casillas de Cónyuge, Concubina(rio) y Otra relación en
`x = 29,6 / 87,9 / 174,5`, `y = 266,2` (9,4 × 9,4 pt).

## 6. Fidelidad de fuentes: qué se puede editar y cómo se avisa

Cada `TextRun` trae (JSON → Dart): `font_kind` → `fontKind` (`embedded_full`, `embedded_subset`,
`standard`, `not_embedded`, `type3`), `font_embedded` → `fontEmbedded`, y `editable` /
`not_editable` → `editable` / `notEditable`. Úsalos para avisar «esta fuente no se puede
reproducir exactamente» (`not_embedded`, `embedded_subset`) y para no ofrecer editar lo que no se puede.

Editar un texto no editable falla con `PdfException.kind == 'not_editable'` y `reason`
(`isNotEditable`): `type3` (glifos que son dibujos), `vertical_writing` (CMap `-V`),
`symbolic_font` (ZapfDingbats) o `rtl_text` (árabe, hebreo: se lee, pero escribirlo exige
formas contextuales). La edición se rechaza entera: nada cambia y no queda paso de deshacer.
Leer, mover, alinear y buscar siguen funcionando; `replaceAll` los deja como están y los cuenta en `skipped`.

Soporte por tipo de fuente (probado con fixtures en `pdfcore/tests/fixtures`, auditoría con
`python3 tools/audit_fonts.py`):

| Fuente | Lee | Reemplaza | Guarda sin editar |
|---|---|---|---|
| Type1/TrueType/MMType1 sin incrustar, WinAnsi | sí | sí; lo que la codificación no trae → fuente de respaldo (avisado) | idéntico |
| Sin incrustar con **MacRoman** o **StandardEncoding** | sí, con su tabla (antes se leía como WinAnsi) | sí, con los códigos de su codificación | idéntico |
| **Symbol** | sí (α β γ…) | solo letras que tiene; las latinas → respaldo (avisado), nunca se vuelven griegas | idéntico |
| **ZapfDingbats** | sin texto (los glifos son símbolos) | no (`symbolic_font`) | idéntico |
| **Type3** | texto = letra del código, como otros lectores | no (`type3`) | idéntico |
| TrueType simple / Type0 incrustada, completa | sí | sí | idéntico |
| …subconjunto | sí | letras ausentes → respaldo (avisado) | idéntico |
| Type0 con `CIDToGIDMap` | sí | igual | idéntico |
| OpenType-CFF, CFF desnudo (`Type1C`), Type1 (PFB) | sí; sin `/Encoding` usa la del programa | sí; lo ausente → respaldo | idéntico |
| **Vertical** (`Identity-V`) | sí | no (`vertical_writing`) | idéntico |
| **Derecha a izquierda** | en orden de dibujo (no lógico) | no (`rtl_text`) | idéntico |
| CJK, cirílico, griego (incrustadas) | sí | lo que el subconjunto trae; el resto → respaldo | idéntico |

Nombres de glifo: lista completa de Adobe (4281 nombres, BSD-3), `uniXXXX`, `uXXXX`, ligaduras
con `_` y variantes con `.` (`pdfcore::glyph_name_text`). No leídas todavía: MacExpertEncoding.

### Letras que la fuente no tiene: orden de intentos y aviso

Al escribir una letra que la fuente del texto no dibuja, el motor prueba, en este orden:

1. **La propia fuente** (embebida completa, o un código que ya dibuja).
2. **La misma fuente ampliada**: si el lector dibuja la fuente con su copia (las estándar y las no
   embebidas) se añade la letra a su `/Differences` y a `/Widths`; si es una fuente compuesta
   TrueType completa se añade el glifo de su programa a `/W` y a `ToUnicode`. Es exactamente la misma cara.
   Solo se hace con letras latinas (Latin-1 y Latin Extendido-A y la puntuación usual), que cualquier
   copia del lector tiene; el cirílico o el griego no se enseñan a una fuente latina.
3. **Una fuente incluida** (con `allowFallback`): Liberation Sans/Serif/Mono (clones métricos de
   Arial, Times y Courier), Carlito (Calibri) y Caladea (Cambria), y para todo lo demás Open Sans (sans) y
   Noto Serif (serifa), todas OFL. La elegida sale del
   nombre y de los `Flags` de la fuente; `similarity` va de 0 a 100 (100 = clon métrico; menos de 90 =
   otra cara: **avisa a la persona**).
4. Si ninguna la tiene: `missing_glyphs` y no se cambia nada.

Cada resultado de edición trae `substitutes: [{char, via, font, similarity}]` con `via` =
`encoding` | `font_program` | `bundled_font` (la lista vacía = nada que avisar). Política
(`setSubstitution`): `perChar` (solo esas letras, por defecto), `wholeRun` (todo el texto en una cara, sin
palabras con dos diseños) o `auto` (`wholeRun` salvo clon métrico). Una cara incluida de otro ancho
se estrecha o ensancha con `Tz` (hasta ±15 %) para conservar el ritmo.

Antes de editar: cada `TextRun` trae `substituteFont`/`substituteSimilarity`, y
`requiredFonts()` lista las fuentes del documento con `needed` (no embebida o solo parcial),
`registered` y `source` (una alternativa libre con licencia y URL). `registerFont(bytes)` acepta una
fuente TrueType que la persona pueda usar (se respeta `fsType`; si prohíbe embeber → `rejected`) y
desde ese momento sustituye a su familia con `similarity` 100.

Alfabetos: se leen y se editan los que la fuente del documento dibuje (cirílico, griego, CJK si la
fuente embebida los trae). Las fuentes incluidas cubren latín, griego y cirílico; **no** CJK. Árabe/hebreo
(`rtl_text`) y escritura vertical se leen pero no se editan. La búsqueda entiende ligaduras (`ﬁ` = `fi`).

## 7. Boceto de pantalla de edición (sin compilar)

Muestra la idea: imagen de fondo + un `Positioned` por texto, con `display * zoom`.

```dart
class EditablePage extends StatelessWidget {
  const EditablePage({super.key, required this.page, required this.image,
      required this.zoom, required this.onEdit});
  final PageData page;          // del motor
  final ImageProvider image;    // de pdfx / PdfRenderer, a width*zoom
  final double zoom;
  final void Function(TextRun run) onEdit;

  @override
  Widget build(BuildContext context) => SizedBox(
        width: page.width * zoom,
        height: page.height * zoom,
        child: Stack(children: [
          Positioned.fill(child: Image(image: image, fit: BoxFit.fill)),
          for (final run in page.runs.where((r) => !r.invisible))
            Positioned(
              left: run.display[0] * zoom,
              top: run.display[1] * zoom,
              width: (run.display[2] - run.display[0]) * zoom,
              height: (run.display[3] - run.display[1]) * zoom,
              child: GestureDetector(behavior: HitTestBehavior.translucent, onTap: () => onEdit(run)),
            ),
        ]),
      );
}
```

Al tocar un texto abre un `TextField` con `run.text`; al confirmar:

```dart
try {
  await session.replaceText(page, run, nuevo);
} on PdfException catch (e) {
  if (e.isMissingGlyphs && await preguntarRespaldo(e.chars)) {
    await session.replaceText(page, run, nuevo, allowFallback: true);
  } else { rethrow; }
}
page = await session.page(pageIndex);          // ids renovados
imagen = await redibujar(await session.save());
```

Lo que **no** trae el boceto y tendrás que resolver en la app: el arrastre del
dedo para colocar el cursor entre letras (la lupa de la versión web), el teclado
que redimensiona la pantalla, y el zoom al ancho. La versión web (`app/static/js/`)
tiene resueltos esos tres casos y sirve de referencia.

## Medir en el dispositivo

Los tiempos de abajo (README) son de Linux. Lo que cuesta en tu tablet solo se sabe midiéndolo allí:

```dart
import 'package:pdfcore/benchmark.dart';
final resultado = benchmarkDocument(bytesDeUnPdfReal, repeat: 3); // tiempos en ms y memoria en MB
debugPrint(jsonEncode(resultado));
```

Mide abrir, leer página, bloques, ítems, líneas, una edición con deshacer, guardar y guardar incremental,
con la mejor de `repeat` pasadas (la primera paga la carga fría). Desde una terminal:
`dart run tool/benchmark.dart archivo.pdf ruta/libpdfcore_ffi.so`.

## Qué no se ha podido verificar (resumen)

- Compilación, enlazado y ejecución en Android/iOS reales (no hay NDK, Xcode ni teléfono donde se escribió).
- Tiempos y memoria en la tablet (use `benchmarkDocument`).
- Cómo ven el resultado los visores de Android (Google Drive, Samsung, Chrome, Acrobat): las comprobaciones
  visuales se hicieron con MuPDF, que es un solo lector.
- PDF con fuentes que no están en las pruebas (solo hay las que generan los scripts de `tools/` y tus tres
  PDF reales): cada fuente rara es un caso nuevo.
- Fuentes del sistema del dispositivo: el motor no las ve; la app debe pasarlas con `registerFont`.
- OCR: el motor solo dice qué páginas lo necesitan.

## Limitaciones del motor a tener presentes

- Texto dentro de formularios XObject: se lee y **se edita en el sitio** (con la propia fuente del formulario;
  nada se mueve y no hay fuente de respaldo). Si el formulario lo dibujan varias páginas, todas muestran el cambio
  (`shared_form`). Mover y borrar texto de un XObject sigue sin estar soportado; censurarlo se rechaza.
- Fuentes incrustadas: los glifos se verifican en el programa de fuente (TrueType, OpenType, CFF y Type 1); lo que no tiene va a la fuente de respaldo. Una fuente cuyo programa no se puede leer es la excepción: ahí solo se escribe lo que la página ya dibujaba.
- PDF cifrados con `/Encrypt` escrito en línea (MuPDF): se abren (se reescribe como objeto) si el archivo usa tabla xref clásica;
  con xref comprimida se informa (`unsupported`).
- Árabe, hebreo y escritura vertical se leen pero no se editan; las fuentes incluidas no cubren CJK.
- Las anotaciones se crean con apariencia propia; no se editan las que traen otros programas salvo mover, borrar y cambiar su texto.
- La censura no toca metadatos ni revisiones anteriores de un archivo guardado de forma incremental, y no edita los píxeles de las imágenes.
