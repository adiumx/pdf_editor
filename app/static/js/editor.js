/**
 * The editor: document state, tools, and turning what the user did into
 * operations the server can apply.
 *
 * Nothing here knows how a PDF works. An edit is described in the coordinates
 * the server handed out, posted, and then read back — the server's re-render is
 * always the truth, so the screen cannot drift away from the file.
 */

import { api } from './api.js';
import { PageView } from './page.js';
import { familyOf } from './fontmap.js';
import { reportWarnings, setStatus, toast, withBusy } from './ui.js';

const DEFAULT_NEW_TEXT = { size: 12, color: '#000000', family: 'sans', bold: false, italic: false };

export class Editor {
  constructor(root) {
    this.root = root;
    this.doc = null;
    this.zoom = 1.5;
    this.revision = 0;
    this.tool = 'select';
    this.pages = new Map();
    this.active = null;
    this.pendingImage = null;
    this.families = [];
    this.listeners = new Set();
  }

  /* ---------- state ---------- */

  onChange(listener) {
    this.listeners.add(listener);
  }

  _emit() {
    for (const listener of this.listeners) listener(this);
  }

  get isOpen() {
    return this.doc !== null;
  }

  /* ---------- document lifecycle ---------- */

  async open(file) {
    await withBusy('Abriendo el PDF…', async () => {
      const state = await api.upload(file);
      this.close({ silent: true });
      this.doc = state;
      this.revision = 0;
      const { families } = await api.fonts(state.id);
      this.families = families;
      await this._buildPages();
    });
    setStatus(`${this.doc.page_count} página${this.doc.page_count === 1 ? '' : 's'}`);
    this._emit();
  }

  close({ silent = false } = {}) {
    if (this.doc) api.close(this.doc.id);
    this.doc = null;
    this.pages.clear();
    this.root.replaceChildren();
    this.active = null;
    if (!silent) {
      setStatus('');
      this._emit();
    }
  }

  async _buildPages() {
    this.root.replaceChildren();
    this.pages.clear();
    for (const geometry of this.doc.pages) {
      const view = new PageView(geometry.page, geometry, {
        onSpanActivate: (...args) => this.activateSpan(...args),
        onBlankClick: () => this.commitActive(),
        onMarquee: (...args) => this.handleMarquee(...args),
      });
      view.setTool(this.tool);
      this.pages.set(geometry.page, view);
      this.root.append(view.element);
    }
    await this.refreshPages();
  }

  /** Re-render pages and reload their text. Omit `only` to do all of them. */
  async refreshPages(only = null) {
    if (!this.doc) return;
    const numbers = only ?? [...this.pages.keys()];
    await Promise.all(
      numbers.map(async (pno) => {
        const view = this.pages.get(pno);
        if (!view) return;
        const geometry = this.doc.pages[pno] || view.geometry;
        view.setGeometry(geometry, this.zoom, api.renderUrl(this.doc.id, pno, this.zoom, this.revision));
        const page = await api.pageText(this.doc.id, pno);
        view.setLines(page.lines);
      }),
    );
  }

  /* ---------- operations ---------- */

  async applyOperations(operations, { affected = null, structural = false } = {}) {
    if (!this.doc || !operations.length) return;
    await withBusy('Aplicando cambios…', async () => {
      const result = await api.operations(this.doc.id, operations);
      this.revision += 1;
      this.doc = { ...this.doc, ...result };
      reportWarnings(result.warnings);
      if (structural) {
        // Pages moved, were added or removed: indices the views hold are stale.
        this.doc = await api.document(this.doc.id);
        await this._buildPages();
      } else {
        await this.refreshPages(affected);
      }
    });
    this._emit();
  }

  async undo() {
    if (!this.doc?.can_undo) return;
    await withBusy('Deshaciendo…', async () => {
      const state = await api.undo(this.doc.id);
      this.revision += 1;
      this.doc = { ...this.doc, ...state };
      await this._buildPages();
    });
    this._emit();
  }

  async redo() {
    if (!this.doc?.can_redo) return;
    await withBusy('Rehaciendo…', async () => {
      const state = await api.redo(this.doc.id);
      this.revision += 1;
      this.doc = { ...this.doc, ...state };
      await this._buildPages();
    });
    this._emit();
  }

  /* ---------- tools ---------- */

  async setTool(tool) {
    await this.commitActive();
    this.tool = tool;
    for (const view of this.pages.values()) view.setTool(tool);
    this._emit();
  }

  async setZoom(zoom) {
    await this.commitActive();
    this.zoom = zoom;
    await this.refreshPages();
  }

  /* ---------- editing an existing span ---------- */

  async activateSpan(view, line, spanIndex, host, event) {
    if (this.tool !== 'select') return;
    if (this.active?.line?.id === line.id && this.active.spanIndex === spanIndex) return;
    // Read the click position now: the edit below is asynchronous and the event
    // is stale by the time the caret needs placing.
    const clickedAt = event ? { x: event.clientX, y: event.clientY } : null;
    await this.commitActive();

    const input = view.openEditor(line, spanIndex);
    if (!input) return;
    const span = line.spans[spanIndex];
    this.active = {
      view,
      line,
      spanIndex,
      input,
      host,
      original: span.text,
      format: {
        family: null,
        size: span.size,
        color: span.color,
        bold: span.bold,
        italic: span.italic,
        align: line.align || 'left',
        fit: false,
      },
      formatDirty: false,
    };

    input.focus();
    this._placeCaret(input, clickedAt);
    input.addEventListener('keydown', (event) => this._onEditorKey(event));
    this._emit();
  }

  /** Put the caret where the user clicked, or at the end if that is unknown. */
  _placeCaret(element, at) {
    const selection = window.getSelection();
    let range = null;
    if (at) {
      if (document.caretRangeFromPoint) {
        range = document.caretRangeFromPoint(at.x, at.y);
      } else if (document.caretPositionFromPoint) {
        const position = document.caretPositionFromPoint(at.x, at.y);
        if (position) {
          range = document.createRange();
          range.setStart(position.offsetNode, position.offset);
        }
      }
      if (range && !element.contains(range.startContainer)) range = null;
    }
    if (!range) {
      range = document.createRange();
      range.selectNodeContents(element);
      range.collapse(false);
    }
    selection.removeAllRanges();
    selection.addRange(range);
  }

  _onEditorKey(event) {
    if (event.key === 'Escape') {
      event.preventDefault();
      this.cancelActive();
    } else if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      this.commitActive();
    }
  }

  /** Update the live preview when the format bar changes. */
  updateActiveFormat(patch) {
    if (!this.active) return;
    Object.assign(this.active.format, patch);
    this.active.formatDirty = true;
    const { input, format } = this.active;
    if ('size' in patch) input.style.fontSize = `${format.size * this.zoom}px`;
    if ('color' in patch) input.style.color = format.color;
    if ('bold' in patch) input.style.fontWeight = format.bold ? '700' : '400';
    if ('italic' in patch) input.style.fontStyle = format.italic ? 'italic' : 'normal';
    if ('family' in patch && format.family) {
      input.style.fontFamily = ['sans', 'serif', 'mono'].includes(format.family)
        ? { sans: 'Arial, sans-serif', serif: 'Georgia, serif', mono: 'Menlo, monospace' }[format.family]
        : `"${format.family}", "${familyOf(format.family)}", sans-serif`;
    }
    this._emit();
  }

  cancelActive() {
    if (!this.active) return;
    if (this.active.finish) {
      this.active.finish(false);
      return;
    }
    const { view, line, spanIndex } = this.active;
    view.closeEditor(line.id, spanIndex);
    this.active = null;
    this._emit();
  }

  /** Write the active span back into the PDF, if anything actually changed. */
  async commitActive() {
    if (!this.active) return;
    // A new text box is committed by its own handler, which knows the
    // rectangle it was drawn in; there is no line to write back to.
    if (this.active.finish) {
      await this.active.finish(true);
      return;
    }
    const { view, line, spanIndex, input, original, format, formatDirty } = this.active;
    const text = input.textContent.replace(/\n/g, ' ');
    this.active = null;
    view.closeEditor(line.id, spanIndex);

    if (text === original && !formatDirty) {
      this._emit();
      return;
    }

    const spans = line.spans.map((span, index) => {
      if (index !== spanIndex) return { ...span };
      const updated = { ...span, text, size: format.size, color: format.color };
      if (formatDirty) {
        updated.bold = format.bold;
        updated.italic = format.italic;
        // A style or family change has to go through family resolution; a pure
        // text edit must not, or it would lose the exact embedded program.
        updated.family = format.family || familyOf(span.font);
      }
      return updated;
    });

    const kept = spans.filter((span) => span.text !== '');
    const operation = {
      op: 'replace_line',
      page: line.page,
      bbox: line.bbox,
      origin: line.origin,
      rotation: line.rotation,
      align: format.align,
      fit: format.fit ? 'shrink' : 'overflow',
      // Everything before the edited span is unchanged, so the server can leave
      // it on the page untouched instead of redrawing it.
      from_span: kept.indexOf(spans[spanIndex]),
      spans: kept,
    };
    await this.applyOperations([operation], { affected: [line.page] });
  }

  /** Remove the span being edited (or the whole line if it is the only one). */
  async deleteActive() {
    if (!this.active) return;
    const { view, line, spanIndex } = this.active;
    this.active = null;
    view.closeEditor(line.id, spanIndex);
    const remaining = line.spans.filter((_span, index) => index !== spanIndex);
    const operation = remaining.length
      ? {
          op: 'replace_line',
          page: line.page,
          bbox: line.bbox,
          origin: line.origin,
          rotation: line.rotation,
          align: line.align,
          spans: remaining,
        }
      : { op: 'delete_line', page: line.page, bbox: line.bbox, origin: line.origin, rotation: line.rotation };
    await this.applyOperations([operation], { affected: [line.page] });
  }

  /* ---------- new content ---------- */

  async handleMarquee(view, tool, rect, start) {
    const width = rect[2] - rect[0];
    const height = rect[3] - rect[1];
    if (tool === 'text') {
      const box = width < 8 || height < 8
        ? [start.x, start.y, start.x + 220, start.y + DEFAULT_NEW_TEXT.size * 1.6]
        : rect;
      await this.commitActive();
      this._openNewTextBox(view, box);
    } else if (tool === 'erase') {
      if (width < 2 || height < 2) return;
      await this.applyOperations([{ op: 'erase_area', page: view.pageNumber, rect }], {
        affected: [view.pageNumber],
      });
    } else if (tool === 'image') {
      if (!this.pendingImage) {
        toast('Elige primero una imagen con el botón «Imagen».', 'warn');
        return;
      }
      const asset = this.pendingImage;
      const box = width < 8 || height < 8
        ? [start.x, start.y, start.x + 200, start.y + (200 * asset.height) / asset.width]
        : rect;
      await this.applyOperations(
        [{ op: 'insert_image', page: view.pageNumber, rect: box, asset: asset.asset }],
        { affected: [view.pageNumber] },
      );
    }
  }

  /** A floating textarea that becomes an `add_text` operation when confirmed. */
  _openNewTextBox(view, rect) {
    const z = this.zoom;
    const box = document.createElement('div');
    box.className = 'newbox';
    Object.assign(box.style, {
      left: `${rect[0] * z}px`,
      top: `${rect[1] * z}px`,
      width: `${(rect[2] - rect[0]) * z}px`,
      height: `${(rect[3] - rect[1]) * z}px`,
    });

    const input = document.createElement('textarea');
    input.className = 'newbox__input';
    input.placeholder = 'Escribe aquí… (Ctrl+Enter para insertar)';
    Object.assign(input.style, {
      fontSize: `${DEFAULT_NEW_TEXT.size * z}px`,
      fontFamily: 'Arial, sans-serif',
      color: DEFAULT_NEW_TEXT.color,
    });
    box.append(input);
    view.layer.append(box);
    input.focus();

    const format = { ...DEFAULT_NEW_TEXT, align: 'left', fit: false };
    this.active = {
      view,
      newBox: { box, input, rect, format },
      format,
      formatDirty: false,
      input,
      line: null,
      spanIndex: null,
    };
    this._emit();

    let finished = false;
    const finish = async (commit) => {
      // Committing removes the box, which blurs the textarea and would call
      // this a second time.
      if (finished) return;
      finished = true;
      const text = input.value;
      box.remove();
      if (this.active?.newBox?.box === box) this.active = null;
      this._emit();
      if (!commit || !text.trim()) return;
      await this.applyOperations(
        [
          {
            op: 'add_text',
            page: view.pageNumber,
            rect,
            text,
            size: format.size,
            color: format.color,
            family: format.family,
            bold: format.bold,
            italic: format.italic,
            align: format.align,
          },
        ],
        { affected: [view.pageNumber] },
      );
      this.setTool('select');
    };

    input.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        finish(false);
      } else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        finish(true);
      }
    });
    input.addEventListener('blur', () => {
      // Reaching for the format bar is still editing this box; only focus
      // landing anywhere else means the user is done with it.
      setTimeout(() => {
        if (document.activeElement?.closest?.('#formatbar')) return;
        finish(true);
      }, 0);
    });
    this.active.finish = finish;
  }

  async chooseImage(file) {
    if (!this.doc) return;
    const asset = await withBusy('Subiendo la imagen…', () => api.uploadAsset(this.doc.id, file));
    this.pendingImage = asset;
    toast('Ahora dibuja un rectángulo en la página para colocar la imagen.');
    this.setTool('image');
  }

  /* ---------- pages ---------- */

  async pageOperation(operation) {
    await this.commitActive();
    await this.applyOperations([operation], { structural: true });
  }

  download() {
    if (!this.doc) return;
    const link = document.createElement('a');
    link.href = api.downloadUrl(this.doc.id);
    link.download = this.doc.name.endsWith('.pdf') ? this.doc.name : `${this.doc.name}.pdf`;
    document.body.append(link);
    link.click();
    link.remove();
  }
}
