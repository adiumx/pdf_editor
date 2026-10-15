# Usar el motor en Flutter

Guía para llevar `pdfcore` a una app Flutter (Android ahora, iOS después).

## Qué está probado y qué no

| Pieza | Estado |
|---|---|
| Motor Rust (`pdfcore`) | Probado: 145 pruebas, fuzz con ~25 mil archivos dañados, y contra PDF reales de seguros |
| Capa C (`pdfcore-ffi`) | Probada llamándola desde Python (`ctypes`) y desde Dart |
| Paquete Dart (`engine/dart`) | **Probado**: 28 pruebas con `dart test` contra la librería real, en Linux. Una versión anterior ya se integró y probó en una app Flutter en Windows y en un emulador Android (lectura, edición, formularios, guardado) |
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
Si pesa demasiado, el siguiente ahorro es subconjuntar las fuentes incrustadas
(~3 MB de Liberation van dentro).

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

## 6. Boceto de pantalla de edición (sin compilar)

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

## Limitaciones del motor a tener presentes

- Texto dentro de formularios XObject: se lee, no se edita.
- Fuentes incrustadas: los glifos se verifican en el programa de fuente (TrueType, OpenType, CFF y Type 1); lo que no tiene va a la fuente de respaldo. Una fuente cuyo programa no se puede leer es la excepción: ahí solo se escribe lo que la página ya dibujaba.
- Con texto justificado, el reajuste solo puede mover tramos enteros (no reparte el espacio entre palabras): si hay un `exceeds_margin`, revisa la línea.
- Faltan las anotaciones (resaltar, notas).
- PDF cifrados con el diccionario `/Encrypt` escrito en línea (lo hace MuPDF): se informa, no se abre.
- Párrafos: no se reconoce texto justificado ni con sangría de primera línea, y no se empuja lo que hay debajo.
