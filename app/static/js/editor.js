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
import { alignmentDeltas } from './align.js';
import { familyOf } from './fontmap.js';
import { reportWarnings, setStatus, toast, withBusy } from './ui.js';

/** A block is its paragraph, so its first line's id names it. */
const blockKey = (block) => block[0]?.id || '';

/** The box enclosing a run of lines. */
function boxOf(lines) {
  const rects = lines.map((line) => line.bbox);
  return [
    Math.min(...rects.map((r) => r[0])),
    Math.min(...rects.map((r) => r[1])),
    Math.max(...rects.map((r) => r[2])),
    Math.max(...rects.map((r) => r[3])),
  ];
}

const DEFAULT_NEW_TEXT = { size: 12, color: '#000000', family: 'sans', bold: false, italic: false };

export class Editor {
  constructor(root) {
    this.root = root;
    this.doc = null;
    this.zoom = 1.5;
    this.revision = 0;
    this.tool = 'select';
    this.markKind = 'highlight';
    this.markColor = '#ffd83d';
    this.pages = new Map();
    this.active = null;
    this.pendingImage = null;
    this.families = [];
    this.listeners = new Set();
    this.grid = null;
    // The block the arrow keys would move: picked by clicking it with the move
    // tool, without dragging.
    this.picked = null;
    // Edits the user has not downloaded yet. The document lives in the
    // server's memory and nowhere else, so leaving the page loses them.
    this.dirty = false;
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

  /** Whether leaving now would lose work. */
  get hasUnsavedWork() {
    return this.isOpen && this.dirty;
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
      this.dirty = false;
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
        onMoveStart: (...args) => this.startMove(...args),
        snapPoint: (view, x, y) => this.grid?.snapPoint(view, x, y) || { x, y, marks: [] },
        measure: (dx, dy) => this.grid?.format(dx, dy) || '',
        onBlankClick: () => this.commitActive(),
        onMarquee: (...args) => this.handleMarquee(...args),
        removeMark: (...args) => this.removeMark(...args),
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
        view.setMarks(page.marks);
        this.grid?.paint(view);
        if (this.picked?.view === view) this.clearPick();
      }),
    );
  }

  /* ---------- operations ---------- */

  async applyOperations(operations, { affected = null, structural = false } = {}) {
    if (!this.doc || !operations.length) return;
    await withBusy('Aplicando cambios…', async () => {
      const result = await api.operations(this.doc.id, operations);
      this.revision += 1;
      this.dirty = true;
      // An edit can add a page on its own: text pushed past the foot of the
      // sheet is carried onto a continuation page. Nothing asked for that, so
      // it has to be noticed here rather than declared up front.
      const grew = result.page_count !== this.doc.pages.length;
      this.doc = { ...this.doc, ...result };
      reportWarnings(result.warnings);
      if (structural || grew) {
        // Pages moved, were added or removed: indices the views hold are stale.
        this.doc = await api.document(this.doc.id);
        await this._buildPages();
      } else {
        await this.refreshPages(affected);
      }
    });
    this._emit();
  }

  /**
   * Take in a change the server made on its own, such as a replace-all.
   *
   * The pages have to be rebuilt rather than refreshed: a replacement can
   * re-break paragraphs anywhere in the document.
   */
  async afterExternalEdit(result) {
    if (!this.doc) return;
    this.revision += 1;
    this.dirty = true;
    this.doc = { ...this.doc, ...result };
    reportWarnings(result.warnings);
    await withBusy('Actualizando…', async () => {
      this.doc = await api.document(this.doc.id);
      await this._buildPages();
    });
    this._emit();
  }

  /** Pages that are pictures of text, with no text of their own yet. */
  get scannedPages() {
    return (this.doc?.pages || []).filter((page) => page.needs_ocr).map((page) => page.page);
  }

  /** Whether this machine can read text off a picture at all. */
  get canRecognise() {
    return Boolean(this.doc?.ocr?.available);
  }

  /**
   * Read the text off the scanned pages so they can be edited.
   *
   * Slow by nature: every page is rendered and handed to the recogniser.
   */
  async recognise(pages = null) {
    if (!this.doc) return 0;
    const result = await withBusy('Reconociendo el texto… puede tardar un poco', () =>
      api.ocr(this.doc.id, pages ? { pages } : {}),
    );
    if (result.recognised?.length) {
      this.revision += 1;
      this.dirty = true;
      this.doc = result;
      await this._buildPages();
    } else {
      this.doc = { ...this.doc, ...result };
    }
    this._emit();
    return result.recognised?.length || 0;
  }

  async undo() {
    if (!this.doc?.can_undo) return;
    await withBusy('Deshaciendo…', async () => {
      const state = await api.undo(this.doc.id);
      this.revision += 1;
      this.dirty = true;
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
      this.dirty = true;
      this.doc = { ...this.doc, ...state };
      await this._buildPages();
    });
    this._emit();
  }

  /* ---------- tools ---------- */

  async setTool(tool) {
    await this.commitActive();
    if (tool !== 'move') this.clearPick();
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
        fit: 'overflow',
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
  async commitActive({ reflow = false } = {}) {
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

    if (text === original && !formatDirty && !reflow) {
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
    const operation = this._writeOperation(view, line, kept, spanIndex, format, reflow);
    await this.applyOperations([operation], { affected: [line.page] });
  }

  /**
   * How to put a line's new spans back on the page.
   *
   * Text that runs across several lines is sent as its whole paragraph, so the
   * server can re-break it: a line that no longer fits should push words down,
   * not run into the margin. A standalone line is sent on its own, which lets
   * the server leave everything before the edit untouched.
   */
  _writeOperation(view, line, spans, spanIndex, format, reflow) {
    const paragraph = view.lines.filter((item) => item.paragraph === line.paragraph);
    const base = {
      page: line.page,
      align: format.align,
      fit: format.fit || 'overflow',
    };

    if (paragraph.length > 1) {
      return {
        ...base,
        op: 'replace_paragraph',
        // The paragraph's own box, widened to the column it sits in: its widest
        // line is not necessarily how far the text is allowed to run.
        box: line.block_bbox,
        // How far the text may run, as a point on the page: for text that does
        // not read left to right, a margin is not simply the box's right edge.
        measure_point: line.measure_point,
        reflow,
        lines: paragraph.map((item) => (item.id === line.id ? { ...item, spans } : item)),
        edited: {
          line: paragraph.findIndex((item) => item.id === line.id),
          span: spans.indexOf(spans[spanIndex]) >= 0 ? spanIndex : 0,
        },
      };
    }

    return {
      ...base,
      op: 'replace_line',
      bbox: line.bbox,
      origin: line.origin,
      rotation: line.rotation,
      // Everything before the edited span is unchanged, so the server can leave
      // it on the page untouched instead of redrawing it.
      from_span: spanIndex,
      spans,
    };
  }

  /** Whether the text being edited is part of a paragraph that can re-wrap. */
  get activeIsInParagraph() {
    const active = this.active;
    if (!active?.line) return false;
    return active.view.lines.filter((item) => item.paragraph === active.line.paragraph).length > 1;
  }

  /** Remove the span being edited (or the whole line if it is the only one). */
  async deleteActive() {
    if (!this.active) return;
    const { view, line, spanIndex } = this.active;
    this.active = null;
    view.closeEditor(line.id, spanIndex);
    const remaining = line.spans.filter((_span, index) => index !== spanIndex);
    const operation = remaining.length
      ? this._writeOperation(
          view, line, remaining, 0,
          { align: line.align, fit: 'overflow' }, true,
        )
      : { op: 'delete_line', page: line.page, bbox: line.bbox, origin: line.origin, rotation: line.rotation };
    await this.applyOperations([operation], { affected: [line.page] });
  }

  /**
   * Drag a block of text to another place on the page.
   *
   * What moves is the paragraph the grabbed line belongs to, because a line
   * pulled out of its paragraph leaves a hole behind it.
   */
  startMove(view, line, event) {
    if (this.tool !== 'move') return;
    const grabbed = view.lines.filter((item) => item.paragraph === line.paragraph);
    // Grabbing one of several picked blocks drags the whole selection: having
    // lined them up, moving them apart again by accident would be absurd.
    const dragging = this.isPicked(grabbed) && this.picked.view === view
      ? this.picked.blocks
      : [grabbed];
    const block = dragging.flat();
    const rects = block.map((item) => item.bbox);
    const box = boxOf(block);
    const start = view.toPagePoint(event);
    view.element.classList.add('is-dragging');
    view.showGhost(rects, 0, 0);

    const settle = (someEvent) => {
      const at = view.toPagePoint(someEvent);
      const raw = { dx: at.x - start.x, dy: at.y - start.y };
      if (!this.grid) return { ...raw, marks: [], at };
      return {
        ...this.grid.snapDelta(view, box, raw.dx, raw.dy, {
          constrain: someEvent.shiftKey,
          exclude: block,
        }),
        at,
      };
    };

    const move = (moveEvent) => {
      const { dx, dy, marks, at } = settle(moveEvent);
      view.showGhost(rects, dx, dy);
      view.showGuides(marks);
      view.showReadout(this.grid?.format(dx, dy) || '', at.x, at.y);
    };

    const up = async (upEvent) => {
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', up);
      view.clearGhost();

      // Whether this was a drag is decided by the hand, not by the snapping:
      // with a grid on, standing still still produces an offset, and a plain
      // click would slide the block onto the nearest line.
      const at = view.toPagePoint(upEvent);
      if (Math.abs(at.x - start.x) < 1 && Math.abs(at.y - start.y) < 1) {
        // A click: pick the block so the arrow keys can move it.
        this.pick(view, grabbed, { add: upEvent.shiftKey || upEvent.ctrlKey || upEvent.metaKey });
        return;
      }

      const { dx, dy } = settle(upEvent);
      await this.applyOperations(
        dragging.map((lines) => ({
          op: 'move_block', page: view.pageNumber, lines, dx, dy,
        })),
        { affected: [view.pageNumber] },
      ).catch(() => {});
    };

    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', up);
  }

  /**
   * Mark a block so the arrow keys and the alignment bar act on it.
   *
   * With `add`, the block joins the selection instead of replacing it, and a
   * block already in it drops out — which is how a mis-click is undone without
   * starting the selection over. A selection lives on one page: aligning a
   * paragraph with something on another sheet means nothing.
   */
  pick(view, block, { add = false } = {}) {
    const key = blockKey(block);
    if (!add || !this.picked || this.picked.view !== view) {
      this.clearPick();
      this.picked = { view, blocks: [block] };
    } else {
      const blocks = this.picked.blocks.filter((other) => blockKey(other) !== key);
      this.picked.blocks = blocks.length === this.picked.blocks.length
        ? [...blocks, block]
        : blocks;
      if (!this.picked.blocks.length) {
        this.clearPick();
        this._emit();
        return;
      }
    }
    view.setPicked(this.pickedLines());
    this._emit();
  }

  clearPick() {
    this.picked?.view.setPicked([]);
    this.picked = null;
  }

  /** Every line of every picked block, flattened. */
  pickedLines() {
    return (this.picked?.blocks || []).flat();
  }

  isPicked(block) {
    const key = blockKey(block);
    return (this.picked?.blocks || []).some((other) => blockKey(other) === key);
  }

  /**
   * Move the picked block by a small step.
   *
   * A point at a time by default, a whole grid step with shift: the two things
   * you want are "just a touch" and "exactly one square".
   */
  async nudge(dx, dy, wide = false) {
    if (!this.picked) return;
    const step = wide ? (this.grid?.spacing || 10) : 1;
    const { view, blocks } = this.picked;
    await this.applyOperations(
      blocks.map((lines) => ({
        op: 'move_block', page: view.pageNumber, lines,
        dx: dx * step, dy: dy * step,
      })),
      { affected: [view.pageNumber] },
    ).catch(() => {});
  }

  /**
   * Line the picked blocks up, or space them out evenly.
   *
   * Aligning takes the outermost edge as the line to meet — the leftmost for
   * "left", the topmost for "top" — because that is the one already on the
   * page: everything moves to a place something is, rather than all of them to
   * a place none of them was. Centring uses the middle of what is selected.
   *
   * Distributing keeps the two outermost blocks where they are and shares the
   * space between them evenly, which is what makes it a way of tidying rather
   * than of moving the group.
   */
  async align(how) {
    if (!this.picked || this.picked.blocks.length < 2) return;
    const { view, blocks } = this.picked;
    const boxes = blocks.map(boxOf);
    const deltas = alignmentDeltas(how, boxes);
    if (!deltas) return;

    const operations = blocks
      .map((lines, index) => ({
        op: 'move_block', page: view.pageNumber, lines,
        dx: deltas[index][0], dy: deltas[index][1],
      }))
      .filter((op) => Math.abs(op.dx) > 0.01 || Math.abs(op.dy) > 0.01);
    if (!operations.length) return;
    await this.applyOperations(operations, { affected: [view.pageNumber] });
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
    } else if (tool === 'mark') {
      await this.addMark(view, rect, start);
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

  /** Put a mark on whatever the drag crossed. */
  async addMark(view, rect, start) {
    const kind = this.markKind || 'highlight';
    let box = rect;
    if (kind === 'note') {
      // A note is a pin, not an area: it goes where the click landed.
      box = [start.x, start.y, start.x + 18, start.y + 18];
    } else if (rect[2] - rect[0] < 3 && rect[3] - rect[1] < 3) {
      // A highlighter is swept along a line, so a drag with no height is the
      // ordinary gesture, not an empty one. Only a click marks nothing.
      return;
    }
    let note = '';
    if (kind === 'note') {
      note = (window.prompt('Texto de la nota:', '') || '').trim();
      if (!note) return;
    }
    await this.applyOperations(
      [{
        op: 'add_mark', page: view.pageNumber, kind, rect: box,
        color: this.markColor || '#ffd83d', note,
      }],
      { affected: [view.pageNumber] },
    );
  }

  /** Take a mark off the page. */
  async removeMark(view, mark) {
    if (this.tool !== 'mark') return;
    await this.applyOperations(
      [{ op: 'delete_mark', page: view.pageNumber, xref: mark.xref }],
      { affected: [view.pageNumber] },
    );
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

    const format = { ...DEFAULT_NEW_TEXT, align: 'left', fit: 'overflow' };
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
    this.dirty = false;
    this._emit();
  }
}
