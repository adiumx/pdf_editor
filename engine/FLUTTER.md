# Usar el motor en Flutter

Guía para llevar `pdfcore` a una app Flutter (Android ahora, iOS después).

## Qué está probado y qué no

| Pieza | Estado |
|---|---|
| Motor Rust (`pdfcore`) | Probado: 60 pruebas, y contra PDF reales de seguros |
| Capa C (`pdfcore-ffi`) | Probada llamándola desde Python (`ctypes`) y desde Dart |
| Paquete Dart (`engine/dart`) | **Probado**: 15 pruebas con `dart test` contra la librería real, en Linux |
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
3. **Campos de solo lectura.** `setField` los rechaza (`kind == 'rejected'`). Muchos
   formularios marcan así por error campos que sí se deben rellenar (las 28 casillas
   del formato de Prudential lo están). Pregunta a la persona y reintenta con
   `ignoreReadOnly: true`.
4. **Texto largo en un campo.** El motor rechaza lo que pase de `maxLen` o meta un
   salto de línea en un campo de una línea; el mensaje dice por qué.
5. **Cada operación es atómica**: si falla, nada cambió y no hay nada que deshacer.
6. **Guardar y volver a dibujar.** El visor del sistema dibuja un archivo, no el
   estado interno: tras cada edición, `session.save()` y vuelve a abrir esa página
   en el visor. Con PDF grandes conviene redibujar solo la página editada.
7. **Firmas digitales.** Editar invalida una firma. El motor todavía no avisa de
   ello (está en la lista de pendientes): hazlo tú comprobando
   `fields()` con `kind == FieldKind.signature`.

## 4. Boceto de pantalla de edición (sin compilar)

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
- Verificación de glifos solo en fuentes TrueType (CFF/Type1 se asumen completas).
- Cambiar la longitud de un texto no desplaza lo que sigue en la misma línea.
- Faltan imágenes, anotaciones (resaltar, notas) y el aviso de firma.
- PDF cifrados con contraseña: no soportados todavía.
