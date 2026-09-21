/** Wiring: toolbar, format bar, page rail, drag & drop, keyboard shortcuts. */

import { Editor } from './editor.js';
import { Grid } from './grid.js';
import { needs as alignNeeds } from './align.js';
import { SearchBar } from './search.js';
import { Thumbnails } from './thumbs.js';
import { familyOf } from './fontmap.js';
import { toast } from './ui.js';

const $ = (id) => document.getElementById(id);

const editor = new Editor($('pages-view'));
const thumbnails = new Thumbnails($('thumbs'), editor);
const grid = new Grid({
  bar: $('gridbar'),
  show: $('grid-show'),
  step: $('grid-step'),
  snapGrid: $('grid-snap'),
  snapGuides: $('grid-guides'),
  close: $('grid-close'),
}, editor);
editor.grid = grid;

// The annotate tool's own bar: which mark the drag makes, and in what colour.
const markBar = {
  bar: $('markbar'),
  kind: $('mark-kind'),
  color: $('mark-color'),
  close: $('mark-close'),
};
markBar.kind.addEventListener('change', () => { editor.markKind = markBar.kind.value; });
markBar.color.addEventListener('input', () => { editor.markColor = markBar.color.value; });
markBar.close.addEventListener('click', () => { editor.setTool('select').catch(reportError); });

// Lining several blocks up. The bar is its own hint: it is up exactly while
// there is more than one block picked, so its buttons always mean something.
for (const button of $('alignbar').querySelectorAll('[data-arrange]')) {
  button.addEventListener('click', () => {
    editor.align(button.dataset.arrange).catch(reportError);
  });
}

$('btn-add-pick').addEventListener('click', () => {
  editor.addToSelection = !editor.addToSelection;
  $('btn-add-pick').classList.toggle('is-active', editor.addToSelection);
});

$('btn-duplicate').addEventListener('click', () => editor.duplicate().catch(reportError));

const search = new SearchBar({
  bar: $('findbar'),
  query: $('find-query'),
  count: $('find-count'),
  next: $('find-next'),
  previous: $('find-prev'),
  matchCase: $('find-case'),
  wholeWord: $('find-word'),
  replacement: $('find-replacement'),
  replaceAll: $('find-replace-all'),
  close: $('find-close'),
}, editor);

const dropzone = $('dropzone');
const fileInput = $('file-input');
const formatbar = $('formatbar');

/* ---------- opening a file ---------- */

const UNSAVED = 'Hay cambios sin guardar. Si sales ahora se pierden.';

async function openFile(file) {
  if (!file) return;
  if (editor.hasUnsavedWork && !confirm(`${UNSAVED}\n\n¿Abrir otro PDF de todos modos?`)) {
    return;
  }
  if (!/\.pdf$/i.test(file.name) && file.type !== 'application/pdf') {
    toast('Ese archivo no es un PDF.', 'error');
    return;
  }
  try {
    await editor.open(file);
    dropzone.classList.add('is-hidden');
    $('doc-name').textContent = file.name;
    // The desktop default would run a page wider than a phone screen, so a
    // narrow one starts fitted instead — the zoom a document opens at is
    // never seen wider than the screen that opened it.
    if (NARROW_SCREEN.matches) {
      $('zoom').value = 'fit';
      await editor.fitWidth();
    }
  } catch (error) {
    toast(String(error.message || error), 'error');
  }
}

$('btn-open').addEventListener('click', () => fileInput.click());
$('dropzone-open').addEventListener('click', () => fileInput.click());
fileInput.addEventListener('change', () => {
  openFile(fileInput.files[0]);
  fileInput.value = '';
});

for (const type of ['dragenter', 'dragover']) {
  document.addEventListener(type, (event) => {
    if (!event.dataTransfer?.types.includes('Files')) return;
    event.preventDefault();
    if (!editor.isOpen) dropzone.classList.add('is-dragover');
  });
}
document.addEventListener('dragleave', (event) => {
  if (event.relatedTarget === null) dropzone.classList.remove('is-dragover');
});
document.addEventListener('drop', (event) => {
  if (!event.dataTransfer?.files.length) return;
  event.preventDefault();
  dropzone.classList.remove('is-dragover');
  openFile(event.dataTransfer.files[0]);
});

/* ---------- toolbar ---------- */

for (const button of document.querySelectorAll('[data-tool]')) {
  button.addEventListener('click', async () => {
    if (button.dataset.tool === 'image') {
      pickImage();
      return;
    }
    await editor.setTool(button.dataset.tool);
  });
}

function pickImage() {
  const picker = document.createElement('input');
  picker.type = 'file';
  picker.accept = 'image/png,image/jpeg,image/gif,image/bmp,image/webp';
  picker.addEventListener('change', async () => {
    if (!picker.files.length) return;
    try {
      await editor.chooseImage(picker.files[0]);
    } catch (error) {
      toast(String(error.message || error), 'error');
    }
  });
  picker.click();
}

$('btn-grid').addEventListener('click', () => grid.toggleBar());
$('btn-find').addEventListener('click', () => search.toggle());
$('btn-ocr').addEventListener('click', () => recogniseScans());

async function recogniseScans() {
  const pages = editor.scannedPages;
  if (!pages.length) return;
  if (!editor.canRecognise) {
    toast(editor.doc?.ocr?.detail || 'El reconocimiento de texto no está disponible.', 'warn', 12000);
    return;
  }
  const many = pages.length > 1;
  const question = many
    ? `Hay ${pages.length} páginas escaneadas. ¿Leer su texto? Puede tardar.`
    : 'Esta página está escaneada. ¿Leer su texto? Puede tardar.';
  if (!confirm(question)) return;
  try {
    const done = await editor.recognise(pages);
    toast(
      done
        ? `Texto reconocido en ${done} ${done === 1 ? 'página' : 'páginas'}. Ya puedes editarlo.`
        : 'No se reconoció texto en esas páginas.',
      done ? 'info' : 'warn',
    );
  } catch (error) {
    toast(String(error.message || error), 'error', 12000);
  }
}
$('btn-undo').addEventListener('click', () => editor.undo().catch(reportError));
$('btn-redo').addEventListener('click', () => editor.redo().catch(reportError));
$('btn-save').addEventListener('click', () => editor.download());
$('btn-add-page').addEventListener('click', () =>
  editor.pageOperation({ op: 'insert_page', at: editor.doc.page_count }).catch(reportError),
);
// A screen narrow enough that the desktop default zoom would run a page
// wider than the screen — a phone held upright, mainly.
const NARROW_SCREEN = window.matchMedia('(max-width: 860px)');

$('zoom').addEventListener('change', (event) => {
  const value = event.target.value;
  if (value === 'fit') {
    editor.fitWidth()?.catch(reportError);
  } else {
    editor.setZoom(Number(value)).catch(reportError);
  }
});

// Rotating the phone, or resizing a narrow window, leaves the fit stale
// until something recomputes it — only worth doing while "Ajustar" is what
// is showing, so a zoom the person chose on purpose is never overridden.
window.addEventListener('resize', () => {
  if (editor.isOpen && $('zoom').value === 'fit') editor.fitWidth()?.catch(reportError);
});

function reportError(error) {
  toast(String(error?.message || error), 'error');
}

/* ---------- format bar ---------- */

const fmt = {
  family: $('fmt-family'),
  size: $('fmt-size'),
  color: $('fmt-color'),
  bold: $('fmt-bold'),
  italic: $('fmt-italic'),
  fit: $('fmt-fit'),
  apply: $('fmt-apply'),
  remove: $('fmt-delete'),
  align: $('fmt-align'),
  reflow: $('fmt-reflow'),
};

// Applying from the bar is the same as pressing Enter in the text.
formatbar.addEventListener('keydown', (event) => {
  if (event.key === 'Enter') {
    event.preventDefault();
    editor.commitActive().catch(reportError);
  } else if (event.key === 'Escape') {
    event.preventDefault();
    editor.cancelActive();
  }
});

let familiesFor = null;

function fillFamilies() {
  fmt.family.replaceChildren();
  const groups = [
    ['Fuentes del documento', editor.families.filter((f) => f.source === 'document')],
    ['Estándar del PDF', editor.families.filter((f) => f.source === 'standard')],
    ['Instaladas en este equipo', editor.families.filter((f) => f.source === 'system')],
    ['Genéricas', editor.families.filter((f) => f.source === 'generic')],
  ];
  const labels = { sans: 'Sans-serif', serif: 'Serif', mono: 'Monoespaciada' };
  for (const [label, items] of groups) {
    if (!items.length) continue;
    const group = document.createElement('optgroup');
    group.label = label;
    for (const item of items) {
      const option = document.createElement('option');
      option.value = item.name;
      option.textContent = labels[item.name] || item.name;
      group.append(option);
    }
    fmt.family.append(group);
  }
}

function positionFormatBar() {
  const active = editor.active;
  if (!active) {
    formatbar.hidden = true;
    return;
  }
  const anchor = (active.host || active.newBox?.box)?.getBoundingClientRect();
  if (!anchor) {
    formatbar.hidden = true;
    return;
  }
  formatbar.hidden = false;
  const bar = formatbar.getBoundingClientRect();
  const margin = 8;
  let top = anchor.top - bar.height - margin;
  if (top < 60) top = anchor.bottom + margin;
  const left = Math.min(
    Math.max(margin, anchor.left),
    window.innerWidth - bar.width - margin,
  );
  formatbar.style.top = `${Math.min(top, window.innerHeight - bar.height - margin)}px`;
  formatbar.style.left = `${left}px`;
}

function syncFormatBar() {
  const active = editor.active;
  if (!active) {
    formatbar.hidden = true;
    return;
  }
  const format = active.format;
  // A control the user is typing in must not be rewritten under them.
  const busy = document.activeElement;
  const currentFamily = format.family || (active.line ? familyOf(active.line.spans[active.spanIndex].font) : 'sans');
  if (![...fmt.family.options].some((option) => option.value === currentFamily)) {
    const option = document.createElement('option');
    option.value = currentFamily;
    option.textContent = currentFamily;
    fmt.family.prepend(option);
  }
  if (busy !== fmt.family) fmt.family.value = currentFamily;
  if (busy !== fmt.size) fmt.size.value = String(format.size);
  if (busy !== fmt.color) fmt.color.value = format.color;
  fmt.bold.classList.toggle('is-active', !!format.bold);
  fmt.italic.classList.toggle('is-active', !!format.italic);
  if (busy !== fmt.fit) fmt.fit.value = format.fit || 'overflow';
  fmt.remove.hidden = !active.line;
  // Only text that spans several lines has anything to re-wrap.
  fmt.reflow.hidden = !editor.activeIsInParagraph;
  for (const button of fmt.align.querySelectorAll('[data-align]')) {
    button.classList.toggle('is-active', button.dataset.align === format.align);
  }
  positionFormatBar();
}

fmt.family.addEventListener('change', () => editor.updateActiveFormat({ family: fmt.family.value }));
fmt.size.addEventListener('input', () => {
  const size = Number(fmt.size.value);
  // An empty or half-typed value ("" on the way to "12") is not a size yet.
  if (Number.isFinite(size) && size > 0) editor.updateActiveFormat({ size });
});
fmt.color.addEventListener('input', () => editor.updateActiveFormat({ color: fmt.color.value }));
fmt.bold.addEventListener('click', () => editor.updateActiveFormat({ bold: !editor.active?.format.bold }));
fmt.italic.addEventListener('click', () => editor.updateActiveFormat({ italic: !editor.active?.format.italic }));
fmt.fit.addEventListener('change', () => editor.updateActiveFormat({ fit: fmt.fit.value }));
for (const button of fmt.align.querySelectorAll('[data-align]')) {
  button.addEventListener('click', () => editor.updateActiveFormat({ align: button.dataset.align }));
}
fmt.remove.addEventListener('click', () => editor.deleteActive().catch(reportError));
fmt.reflow.addEventListener('click', () =>
  editor.commitActive({ reflow: true }).catch(reportError),
);
fmt.apply.addEventListener('click', () => editor.commitActive().catch(reportError));

/* ---------- keyboard ---------- */

document.addEventListener('keydown', (event) => {
  // A single letter is a shortcut only when it is not being typed into
  // something. The search box and the format bar are fields like any other.
  const inField = Boolean(
    event.target?.closest?.('input, select, textarea, [contenteditable]'),
  );
  const typing = editor.active !== null || inField;
  const meta = event.ctrlKey || event.metaKey;

  if (meta && event.key.toLowerCase() === 'z') {
    event.preventDefault();
    (event.shiftKey ? editor.redo() : editor.undo()).catch(reportError);
    return;
  }
  if (meta && event.key.toLowerCase() === 'f' && editor.isOpen) {
    event.preventDefault();
    search.open();
    return;
  }
  if (event.key === 'Escape' && search.isOpen && !typing) {
    event.preventDefault();
    search.close();
    return;
  }
  if (meta && event.key.toLowerCase() === 's') {
    event.preventDefault();
    editor.download();
    return;
  }
  if (meta && event.key.toLowerCase() === 'd' && editor.picked && !typing) {
    event.preventDefault();
    editor.duplicate().catch(reportError);
    return;
  }
  if (meta && event.key === 'Enter' && typing) {
    event.preventDefault();
    editor.commitActive().catch(reportError);
    return;
  }
  if (typing || !editor.isOpen) return;

  if (event.key.toLowerCase() === 'g') {
    event.preventDefault();
    grid.toggleBar();
    return;
  }

  if (editor.picked && event.key.startsWith('Arrow')) {
    event.preventDefault();
    const steps = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    const [dx, dy] = steps[event.key];
    editor.nudge(dx, dy, event.shiftKey).catch(reportError);
    return;
  }
  const shortcuts = { v: 'select', m: 'move', t: 'text', e: 'erase', a: 'mark' };
  const tool = shortcuts[event.key.toLowerCase()];
  if (tool) {
    event.preventDefault();
    editor.setTool(tool).catch(reportError);
  } else if (event.key.toLowerCase() === 'i') {
    event.preventDefault();
    pickImage();
  }
});

/* ---------- reacting to editor state ---------- */

editor.onChange(() => {
  const open = editor.isOpen;
  $('tools').hidden = !open;
  $('save-group').hidden = !open;
  $('pages-rail').hidden = !open;
  dropzone.classList.toggle('is-hidden', open);

  // Offered only while there is a scan to read, which is also the only time
  // it would do anything.
  $('btn-ocr').hidden = !open || editor.scannedPages.length === 0;

  $('btn-undo').disabled = !editor.doc?.can_undo;
  $('btn-redo').disabled = !editor.doc?.can_redo;

  // A signed field's signature does not become valid again by waiting, so
  // this is a standing fact about the document, not a toast that clears
  // itself. Stays up for as long as the document does, editable or not.
  const signed = editor.doc?.signed_fields || [];
  $('signed-banner').hidden = !open || signed.length === 0;
  if (signed.length) {
    $('signed-names').textContent = signed.filter(Boolean).join(', ') || 'sin nombre';
  }

  for (const button of document.querySelectorAll('[data-tool]')) {
    button.classList.toggle('is-active', button.dataset.tool === editor.tool);
  }

  // Refill when the document changes: the list starts with that document's own
  // fonts, and syncFormatBar may have prepended one, so a length check would
  // leave the previous document's fonts in place.
  $('findbar').hidden = $('findbar').hidden || !open;
  $('gridbar').hidden = $('gridbar').hidden || !open;
  // The annotate bar is the tool: it is up exactly while the tool is in hand.
  $('markbar').hidden = !open || editor.tool !== 'mark';
  // Only where it means something: adding to a selection of blocks.
  $('btn-add-pick').hidden = !open || editor.tool !== 'move';
  $('btn-add-pick').classList.toggle('is-active', editor.addToSelection);

  const picked = editor.picked?.blocks?.length || 0;
  // Duplicating needs something picked, but not two of them: one is the
  // ordinary case.
  $('btn-duplicate').hidden = !open || editor.tool !== 'move' || picked === 0;
  $('alignbar').hidden = !open || picked < 2;
  if (picked >= 2) {
    $('align-count').textContent = `${picked} bloques seleccionados`;
    for (const button of $('alignbar').querySelectorAll('[data-arrange]')) {
      button.disabled = picked < alignNeeds(button.dataset.arrange);
    }
  }
  search.refresh();

  if (open) grid.paintAll();

  if (open && familiesFor !== editor.doc.id) {
    familiesFor = editor.doc.id;
    fillFamilies();
  } else if (!open) {
    familiesFor = null;
  }
  syncFormatBar();
  thumbnails.render();
});

// Exposed for the browser tests, which need to ask the editor what it thinks.
window.__editor = editor;

window.addEventListener('resize', positionFormatBar);
$('canvas-area').addEventListener('scroll', positionFormatBar, { passive: true });
window.addEventListener('beforeunload', (event) => {
  // The document only exists in the server's memory, so leaving loses it.
  if (!editor.hasUnsavedWork) return;
  event.preventDefault();
  event.returnValue = UNSAVED; // some browsers still read this
  return UNSAVED;
});

// Closing belongs here, not in beforeunload: that one also fires when the user
// is asked whether to leave and decides to stay.
window.addEventListener('pagehide', () => editor.close({ silent: true }));
