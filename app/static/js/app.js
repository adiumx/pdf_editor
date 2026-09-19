/** Wiring: toolbar, format bar, page rail, drag & drop, keyboard shortcuts. */

import { Editor } from './editor.js';
import { Thumbnails } from './thumbs.js';
import { familyOf } from './fontmap.js';
import { toast } from './ui.js';

const $ = (id) => document.getElementById(id);

const editor = new Editor($('pages-view'));
const thumbnails = new Thumbnails($('thumbs'), editor);

const dropzone = $('dropzone');
const fileInput = $('file-input');
const formatbar = $('formatbar');

/* ---------- opening a file ---------- */

async function openFile(file) {
  if (!file) return;
  if (!/\.pdf$/i.test(file.name) && file.type !== 'application/pdf') {
    toast('Ese archivo no es un PDF.', 'error');
    return;
  }
  try {
    await editor.open(file);
    dropzone.classList.add('is-hidden');
    $('doc-name').textContent = file.name;
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

$('btn-undo').addEventListener('click', () => editor.undo().catch(reportError));
$('btn-redo').addEventListener('click', () => editor.redo().catch(reportError));
$('btn-save').addEventListener('click', () => editor.download());
$('btn-add-page').addEventListener('click', () =>
  editor.pageOperation({ op: 'insert_page', at: editor.doc.page_count }).catch(reportError),
);
$('zoom').addEventListener('change', (event) => {
  editor.setZoom(Number(event.target.value)).catch(reportError);
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
};

// Clicking the bar must not count as leaving the text being edited.
formatbar.addEventListener('mousedown', (event) => event.preventDefault());

let familiesFor = null;

function fillFamilies() {
  fmt.family.replaceChildren();
  const groups = [
    ['Fuentes del documento', editor.families.filter((f) => f.source === 'document')],
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
  const currentFamily = format.family || (active.line ? familyOf(active.line.spans[active.spanIndex].font) : 'sans');
  if (![...fmt.family.options].some((option) => option.value === currentFamily)) {
    const option = document.createElement('option');
    option.value = currentFamily;
    option.textContent = currentFamily;
    fmt.family.prepend(option);
  }
  fmt.family.value = currentFamily;
  fmt.size.value = String(format.size);
  fmt.color.value = format.color;
  fmt.bold.classList.toggle('is-active', !!format.bold);
  fmt.italic.classList.toggle('is-active', !!format.italic);
  fmt.fit.checked = !!format.fit;
  fmt.remove.hidden = !active.line;
  for (const button of fmt.align.querySelectorAll('[data-align]')) {
    button.classList.toggle('is-active', button.dataset.align === format.align);
  }
  positionFormatBar();
}

fmt.family.addEventListener('change', () => editor.updateActiveFormat({ family: fmt.family.value }));
fmt.size.addEventListener('input', () => {
  const size = Number(fmt.size.value);
  if (size > 0) editor.updateActiveFormat({ size });
});
fmt.color.addEventListener('input', () => editor.updateActiveFormat({ color: fmt.color.value }));
fmt.bold.addEventListener('click', () => editor.updateActiveFormat({ bold: !editor.active?.format.bold }));
fmt.italic.addEventListener('click', () => editor.updateActiveFormat({ italic: !editor.active?.format.italic }));
fmt.fit.addEventListener('change', () => editor.updateActiveFormat({ fit: fmt.fit.checked }));
for (const button of fmt.align.querySelectorAll('[data-align]')) {
  button.addEventListener('click', () => editor.updateActiveFormat({ align: button.dataset.align }));
}
fmt.remove.addEventListener('click', () => editor.deleteActive().catch(reportError));
fmt.apply.addEventListener('click', () => editor.commitActive().catch(reportError));

/* ---------- keyboard ---------- */

document.addEventListener('keydown', (event) => {
  const typing = editor.active !== null;
  const meta = event.ctrlKey || event.metaKey;

  if (meta && event.key.toLowerCase() === 'z') {
    event.preventDefault();
    (event.shiftKey ? editor.redo() : editor.undo()).catch(reportError);
    return;
  }
  if (meta && event.key.toLowerCase() === 's') {
    event.preventDefault();
    editor.download();
    return;
  }
  if (meta && event.key === 'Enter' && typing) {
    event.preventDefault();
    editor.commitActive().catch(reportError);
    return;
  }
  if (typing || !editor.isOpen) return;

  const shortcuts = { v: 'select', t: 'text', e: 'erase' };
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

  $('btn-undo').disabled = !editor.doc?.can_undo;
  $('btn-redo').disabled = !editor.doc?.can_redo;

  for (const button of document.querySelectorAll('[data-tool]')) {
    button.classList.toggle('is-active', button.dataset.tool === editor.tool);
  }

  // Refill when the document changes: the list starts with that document's own
  // fonts, and syncFormatBar may have prepended one, so a length check would
  // leave the previous document's fonts in place.
  if (open && familiesFor !== editor.doc.id) {
    familiesFor = editor.doc.id;
    fillFamilies();
  } else if (!open) {
    familiesFor = null;
  }
  syncFormatBar();
  thumbnails.render();
});

window.addEventListener('resize', positionFormatBar);
$('canvas-area').addEventListener('scroll', positionFormatBar, { passive: true });
window.addEventListener('beforeunload', () => editor.close({ silent: true }));
