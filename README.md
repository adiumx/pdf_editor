# Editor de PDF

Aplicación web local para **editar el texto de un PDF conservando su tipografía
original**. No superpone cajas de texto encima del documento: localiza el
programa de fuente que el propio PDF lleva embebido, borra los glifos antiguos y
vuelve a dibujar el texto nuevo con esa misma fuente, en la misma línea base, con
el mismo tamaño y el mismo color.

Los archivos se procesan en tu equipo y nunca salen de él.

![Editor de PDF](docs/captura.png)

![Buscar y reemplazar](docs/buscar.png)

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
| **Reconocer texto** | Lee el texto de las páginas escaneadas para poder editarlas. Aparece solo cuando hay alguna. |
| **Buscar** | `Ctrl+F` abre la búsqueda: resalta todas las apariciones, salta entre ellas y reemplaza todas de una vez. |
| **Páginas** | Reordena arrastrando las miniaturas, gira, borra o añade páginas en blanco. |

Desde la barra de formato puedes cambiar la fuente, el tamaño, el color, la
negrita, la cursiva y la alineación. El desplegable de fuentes agrupa:

- **Fuentes del documento** — las que el PDF lleva embebidas.
- **Estándar del PDF** — Helvetica, Times y Courier. No hacen falta instalarlas:
  las tiene cualquier visor, así que el archivo no engorda y se ve igual en todas
  partes.
- **Instaladas en este equipo** — todas las que encuentre en tu sistema.
- **Genéricas** — sans-serif, serif y monoespaciada.

Si el desplegable se te queda corto, instala las familias libres más habituales;
además cubren por compatibilidad métrica a las de Microsoft (Arial, Times New
Roman, Calibri, Cambria):

```bash
sudo apt install fonts-liberation fonts-dejavu fonts-crosextra-carlito \
                 fonts-crosextra-caladea fonts-noto-core
```
La casilla **Ajustar** reduce el tamaño para que un texto más largo quepa en el
ancho original.

La búsqueda ignora mayúsculas y acentos por defecto: escribir «computacion»
encuentra «Computación». Los botones `Aa` y `|ab|` exigen coincidencia exacta o
palabra completa. Reemplazar todo es **un solo paso de deshacer**, y el texto
reemplazado se reajusta como cualquier otra edición.

Atajos: `V` editar · `T` texto · `I` imagen · `E` borrar · `Ctrl+F` buscar ·
`Ctrl+Z` / `Ctrl+Mayús+Z` deshacer y rehacer · `Ctrl+S` guardar · `Enter`
aplicar · `Esc` cancelar.

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
4. La sustituta casi nunca es del mismo ancho que la fuente a la que reemplaza
   —Carlito es compatible con Calibri, Liberation Sans con Arial, y entre sí no
   lo son—, así que el texto se **comprime horizontalmente** hasta ocupar
   exactamente lo que ocupaba el original. El factor no se adivina: se mide del
   propio documento, comparando lo que el PDF declara que ocupa su texto con lo
   que necesitaría la sustituta, y se toma la mediana de todo el página. Sin
   esto, una línea reescrita sin cambiarla salía un 11 % más larga y arrastraba
   el reajuste de párrafos que nadie había tocado.
5. Si pides negrita o cursiva y la familia elegida no tiene ese corte instalado,
   se conserva **el corte** y se cambia de familia, no al revés: pedir cursiva y
   obtener texto redondo no es una respuesta. Se te indica el cambio.
6. Se borran los glifos originales con una redacción y se redibuja el texto en la
   línea base original, reutilizando el programa de fuente **byte a byte**, de
   modo que el PDF guardado sigue siendo válido en cualquier visor.

Los cambios de una misma página se agrupan: primero todos los borrados y después
todos los redibujados, para que una edición no se coma el texto que otra acaba de
escribir.

**El texto se reparte entre las líneas.** Si al editar un renglón deja de caber,
el párrafo se vuelve a partir: las palabras sobrantes bajan a las líneas
siguientes en lugar de salirse por el margen. Mientras quepa, no se toca nada
más que el renglón editado. El botón **⤶ Reajustar** fuerza el reparto cuando
quieres recolocar un párrafo que se ha quedado con huecos.

El ancho al que se reparte no es el del párrafo, sino el de **su columna**: lo
que alcanzan los demás párrafos que arrancan en el mismo margen. Un párrafo de
dos líneas cortas mide poco por sí mismo y se reajustaría en una columna más
estrecha que aquella en la que está.

Si al crecer no cabe en el hueco que tiene debajo —que se mide mirando qué hay
realmente ahí, no suponiéndolo—, primero se **comprime el interlineado**, que se
nota mucho menos que cambiar el cuerpo de letra. Si aun así no cabe, el
desplegable **«Si no cabe»** decide qué hacer:

| Opción | Qué hace |
|---|---|
| **avisar** | Deja que el párrafo crezca y te lo dice. |
| **reducir el tamaño** | Encoge la letra hasta que quepa, nunca por debajo de la mitad. |
| **mover lo de abajo** | Desplaza hacia abajo el texto, los filetes y los enlaces que estorban. |

Mover lo de abajo arrastra el texto, los filetes, las curvas, los cuadriláteros
y las imágenes que estén en la misma columna. Se niega, sin tocar nada, cuando
no podría hacerlo con fidelidad: ante un trazo que no sabe recolocar, o si algo
acabaría fuera de la hoja. Media figura en su sitio viejo es peor que un párrafo
que sobresale.

Un párrafo se reconoce por sus propias señales: mismo tipo y tamaño de letra,
el interlineado propio del bloque, el mismo margen izquierdo y —la más
reveladora— que la línea anterior llegara hasta el margen derecho en vez de
quedarse corta, porque una línea corta es donde termina un párrafo. Así un
titular pegado a un cuerpo de texto no se arrastra dentro de él.

**Sólo se redibuja lo que hace falta.** Si editas la parte final de una línea —el
caso típico: «**Habilidades:** Python, SQL…»—, lo que hay antes se queda tal cual
en la página, sin volver a dibujarse. Importa cuando las fuentes del documento no
se pueden reutilizar: si se reescribiera la línea entera, verías cambiar de
tipografía palabras que no tocaste. En una línea centrada o alineada a la derecha
no se puede: al cambiar el ancho se mueve todo, así que ahí sí se rehace entera.

**Los enlaces se conservan.** Borrar el texto de una línea se lleva por delante
los hipervínculos que hubiera encima, así que se guardan antes y se vuelven a
colocar sobre el texto nuevo, manteniendo su posición relativa dentro de la
línea. Si en cambio borras la línea entera, el enlace desaparece con ella: no
queda nada que pulsar.

## PDF escaneados

Un escaneo no tiene texto: tiene una **fotografía** de un texto, y no hay nada
que editar en ella. El botón **🔎 Reconocer texto** aparece cuando el documento
trae páginas así, y les pone encima una capa de texto legible.

A partir de ahí se edita como cualquier otra página, con una diferencia que
importa: al borrar una línea hay que **tapar también los píxeles** de debajo, o
la palabra escaneada seguiría viéndose bajo lo que la sustituye. La zona editada
queda en blanco, que es lo que hace cualquier editor.

El reconocimiento no guarda con qué tipografía estaba escrito el original
—nadie puede saberlo mirando una foto—, así que el texto que escribas usará una
fuente neutra hasta que elijas otra en la barra de formato.

Necesita Tesseract. En Debian o Ubuntu:

```bash
sudo apt install tesseract-ocr tesseract-ocr-spa
```

Si no está instalado, el editor te lo dice en lugar de fallar.

## Limitaciones
- **El ancho de la columna se deduce del propio texto**, porque el PDF no guarda
  márgenes. Es una estimación buena en un documento corriente, pero una página
  con bloques muy dispares en el mismo margen puede confundirla.
- **Mover lo de abajo sólo llega hasta el final de la hoja.** El texto no pasa a
  la página siguiente; si no cabe, se te dice.
- **Se recolocan líneas, rectángulos, curvas, cuadriláteros e imágenes.** Ante
  un recorte o un sombreado, se niega a mover en lugar de dejarlo descolocado.
- **Las fuentes en subconjunto** solo contienen los glifos que el documento usaba.
  Si escribes caracteres que no están, se sustituye la fuente y se avisa. Muchos
  subconjuntos, además, vienen sin tabla de caracteres Unicode —se direccionan
  por índice de glifo— y entonces no se pueden usar para escribir texto nuevo en
  absoluto; en ese caso se recurre a esa misma fuente instalada en tu equipo, o a
  su equivalente métrico.
- **Si el PDF no embebe sus fuentes**, el texto que edites sí quedará embebido.
  Es lo correcto —así se ve igual en cualquier visor—, pero significa que en un
  equipo sin esa fuente instalada el texto editado se verá bien y el resto no.
  Para evitarlo, edita el documento entero o parte de uno que sí embeba.
- **El texto en trazado o en imagen** no es editable, porque no es texto.
- **Un enlace sobrevive a la edición de su línea aunque borres el texto que
  describía**, recolocado proporcionalmente. Se prefiere conservarlo a adivinar
  que ya no hace falta; si sobra, deshaz o bórralo desde otra herramienta.
- Los PDF protegidos con contraseña requieren la contraseña al abrirlos.
- **La búsqueda no cruza saltos de línea.** Una expresión partida entre dos
  renglones no aparece: las palabras están, pero el archivo nunca las unió y
  adivinar dónde va la unión daría resultados que no se ven en la página.
- Los documentos viven en memoria del servidor y se descartan tras dos horas sin
  actividad. Al salir de la página se te avisa si hay cambios sin guardar, pero
  **guarda antes de cerrar**.

## Arquitectura

```
app/
  main.py      API HTTP (FastAPI): subir, renderizar, leer texto, aplicar cambios, guardar
  store.py     Documentos abiertos en memoria, historial de deshacer y limpieza por inactividad
  extract.py   Lectura de la página como líneas y spans editables, con su geometría
  search.py    Búsqueda en el documento y reemplazo
  fonts.py     Localización, validación y reutilización de las fuentes embebidas
  editor.py    Aplicación de las operaciones sobre el PDF
  ocr.py       Reconocimiento del texto de páginas escaneadas
  static/      Interfaz (JavaScript sin dependencias ni compilación)
```

El navegador no sabe nada de PDF: muestra la página renderizada por el servidor,
deja escribir encima y devuelve **operaciones** que describen el cambio. Después
de cada operación vuelve a pedir la página, así que lo que ves siempre es el
archivo real, nunca una simulación.

## Desarrollo

```bash
pip install -r requirements.txt pytest httpx playwright
python tools/make_sample_pdf.py ejemplo.pdf   # PDF de prueba con fuentes embebidas
pytest
```

(Con el entorno virtual activado. Sin él, usa `python3` y `pip3`.)

La suite cubre la resolución de fuentes, la extracción con páginas rotadas, cada
operación de edición y la API completa, incluida la comprobación de que el PDF
guardado sigue llevando la fuente embebida.

`tests/test_ui.py` conduce la interfaz en un navegador real **con ratón y teclado
de verdad**. Existe porque las pruebas que fijan valores por el DOM no detectan
toda una clase de fallo: una barra de formato cuyos controles no se pueden
pulsar las pasa todas. Se omiten solas si no hay Playwright o navegador
instalado; para ejecutarlas:

```bash
pip install playwright && playwright install chromium
```
