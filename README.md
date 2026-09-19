# Editor de PDF

Aplicación web local para **editar el texto de un PDF conservando su tipografía
original**. No superpone cajas de texto encima del documento: localiza el
programa de fuente que el propio PDF lleva embebido, borra los glifos antiguos y
vuelve a dibujar el texto nuevo con esa misma fuente, en la misma línea base, con
el mismo tamaño y el mismo color.

Los archivos se procesan en tu equipo y nunca salen de él.

![Editor de PDF](docs/captura.png)

## Instalación

```bash
git clone https://github.com/adiumx/pdf_editor.git
cd pdf_editor
python3 -m venv .venv && source .venv/bin/activate   # en Windows: py -m venv .venv && .venv\Scripts\activate
pip install -r requirements.txt
```

En Debian y Ubuntu la orden `python` no existe y `venv` viene en un paquete
aparte; si `python3 -m venv` falla, instálalo primero:

```bash
sudo apt install python3-venv
```

Una vez activado el entorno, `python` y `pip` funcionan con normalidad dentro de
él: los proporciona el propio entorno virtual.

## Uso

```bash
python run.py
```

Se abre `http://localhost:8000` en el navegador. Arrastra un PDF a la ventana y
empieza a editar.

Opciones: `python run.py --port 8080 --no-browser`.

## Qué puedes hacer

| Herramienta | Qué hace |
|---|---|
| **Editar** | Haz clic en cualquier texto y escribe encima. Conserva fuente, tamaño y color. |
| **Texto** | Dibuja un rectángulo e inserta texto nuevo con la fuente que elijas. |
| **Imagen** | Sube una imagen y colócala dibujando un rectángulo. |
| **Borrar** | Elimina todo lo que haya dentro de un rectángulo. |
| **Páginas** | Reordena arrastrando las miniaturas, gira, borra o añade páginas en blanco. |

Desde la barra de formato puedes cambiar la fuente —las que el propio documento
embebe, **todas las instaladas en tu equipo** y tres genéricas—, el tamaño, el
color, la negrita, la cursiva y la alineación.
La casilla **Ajustar** reduce el tamaño para que un texto más largo quepa en el
ancho original.

Atajos: `V` editar · `T` texto · `I` imagen · `E` borrar · `Ctrl+Z` / `Ctrl+Mayús+Z`
deshacer y rehacer · `Ctrl+S` guardar · `Enter` aplicar · `Esc` cancelar.

## Cómo conserva la fuente

Al abrir el documento se extrae, de cada página, el programa de cada fuente
embebida (`extract_font`). Cuando editas una línea:

1. Se localiza el programa que dibujó ese texto, comparando nombres de forma
   tolerante: el PDF puede referirse a `LiberationSerif` mientras el programa se
   llama `Liberation Serif Regular`, o llevar el prefijo de subconjunto
   `ABCDEF+`. Los sufijos neutros (`Regular`, `MT`, `PS`…) se ignoran; el peso y
   la inclinación **nunca**, porque distinguen fuentes distintas.
2. Si el PDF **no** embebe esa fuente —cosa habitual: muchos documentos se
   limitan a nombrarla y confían en que el lector la tenga— se busca **esa misma
   fuente instalada en tu equipo**, por nombre. Y si no está, se recurre a su
   equivalente métricamente compatible: Arial → Liberation Sans, Times New Roman
   → Liberation Serif, Calibri → Carlito, y así. Lo que nunca se hace es elegir
   un reemplazo sólo por el estilo, que es como una Arial acaba convertida en
   cualquier otra sans.
3. Se comprueba que esa fuente tenga glifos para lo que has escrito. Si no los
   tiene (por ejemplo, un subconjunto sin `ñ`), se busca otra fuente del
   documento con el mismo estilo, luego una del sistema y, por último, una de las
   14 estándar del PDF — y se te avisa de la sustitución.
4. Si pides negrita o cursiva y la familia elegida no tiene ese corte instalado,
   se conserva **el corte** y se cambia de familia, no al revés: pedir cursiva y
   obtener texto redondo no es una respuesta. Se te indica el cambio.
5. Se borran los glifos originales con una redacción y se redibuja el texto en la
   línea base original, reutilizando el programa de fuente **byte a byte**, de
   modo que el PDF guardado sigue siendo válido en cualquier visor.

Los cambios de una misma página se agrupan: primero todos los borrados y después
todos los redibujados, para que una edición no se coma el texto que otra acaba de
escribir.

## Limitaciones

- **No hace OCR.** Un PDF escaneado sin capa de texto no tiene texto que editar.
- **El reflujo es por línea**, no por párrafo: si alargas mucho una línea, sobresale
  (o se encoge, con «Ajustar»), pero no se reparte en las líneas siguientes.
- **Las fuentes en subconjunto** solo contienen los glifos que el documento usaba.
  Si escribes caracteres que no están, se sustituye la fuente y se avisa.
- **Si el PDF no embebe sus fuentes**, el texto que edites sí quedará embebido.
  Es lo correcto —así se ve igual en cualquier visor—, pero significa que en un
  equipo sin esa fuente instalada el texto editado se verá bien y el resto no.
  Para evitarlo, edita el documento entero o parte de uno que sí embeba.
- **El texto en trazado o en imagen** no es editable, porque no es texto.
- Los PDF protegidos con contraseña requieren la contraseña al abrirlos.
- Los documentos viven en memoria del servidor y se descartan tras dos horas sin
  actividad. **Guarda antes de cerrar.**

## Arquitectura

```
app/
  main.py      API HTTP (FastAPI): subir, renderizar, leer texto, aplicar cambios, guardar
  store.py     Documentos abiertos en memoria, historial de deshacer y limpieza por inactividad
  extract.py   Lectura de la página como líneas y spans editables, con su geometría
  fonts.py     Localización, validación y reutilización de las fuentes embebidas
  editor.py    Aplicación de las operaciones sobre el PDF
  static/      Interfaz (JavaScript sin dependencias ni compilación)
```

El navegador no sabe nada de PDF: muestra la página renderizada por el servidor,
deja escribir encima y devuelve **operaciones** que describen el cambio. Después
de cada operación vuelve a pedir la página, así que lo que ves siempre es el
archivo real, nunca una simulación.

## Desarrollo

```bash
pip install -r requirements.txt pytest httpx
python tools/make_sample_pdf.py ejemplo.pdf   # PDF de prueba con fuentes embebidas
pytest
```

(Con el entorno virtual activado. Sin él, usa `python3` y `pip3`.)

La suite cubre la resolución de fuentes, la extracción con páginas rotadas, cada
operación de edición y la API completa, incluida la comprobación de que el PDF
guardado sigue llevando la fuente embebida.
