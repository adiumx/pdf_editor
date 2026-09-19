/**
 * One rendered page and the editable overlays sitting on top of it.
 *
 * The image is authoritative: it is what the PDF actually looks like. The
 * overlays are positioned from the same coordinates the server extracted the
 * text at, scaled by the zoom, so a span's box sits exactly over its glyphs.
 */

import { applyTypography } from './fontmap.js';

export class PageView {
  constructor(pageNumber, geometry, handlers) {
    this.pageNumber = pageNumber;
    this.geometry = geometry;
    this.handlers = handlers;
    this.lines = [];
    this.marks = [];
    this.zoom = 1.5;
    this._sampler = null;

    this.element = document.createElement('div');
    this.element.className = 'page tool-select';
    this.element.dataset.page = String(pageNumber);

    this.image = document.createElement('img');
    this.image.className = 'page__image';
    this.image.alt = `Página ${pageNumber + 1}`;
    this.image.draggable = false;

    this.layer = document.createElement('div');
    this.layer.className = 'page__layer';

    this.element.append(this.image, this.layer);
    this._bindMarquee();
  }

  /** Size the page box and (re)load its rendered image. */
  setGeometry(geometry, zoom, src) {
    this.geometry = geometry;
    this.zoom = zoom;
    this.element.style.width = `${Math.round(geometry.width * zoom)}px`;
    this.element.style.height = `${Math.round(geometry.height * zoom)}px`;
    this._sampler = null;
    if (src && this.image.getAttribute('src') !== src) this.image.src = src;
  }

  setTool(tool) {
    this.element.className = `page tool-${tool}`;
  }

  /** Rebuild the span overlays from freshly extracted lines. */
  setLines(lines) {
    this.lines = lines;
    this.layer.replaceChildren();
    for (const line of lines) {
      line.spans.forEach((span, index) => {
        this.layer.append(this._spanElement(line, span, index));
      });
    }
    this._renderMarks();
  }

  /** The marks already on the page, as targets to click rather than as paint.
   *
   * What the reader sees is the rendered image, annotations and all; these are
   * only the handles for taking one off again, and they show only while the
   * annotate tool is the one in hand.
   */
  setMarks(marks) {
    this.marks = marks || [];
    this._renderMarks();
  }

  _renderMarks() {
    for (const stale of this.layer.querySelectorAll('.markhit')) stale.remove();
    const z = this.zoom;
    for (const mark of this.marks) {
      const element = document.createElement('div');
      element.className = 'markhit';
      element.dataset.xref = String(mark.xref);
      element.title = mark.note
        ? `${mark.note} — clic para quitarla`
        : 'Clic para quitar la marca';
      Object.assign(element.style, {
        left: `${mark.bbox[0] * z}px`,
        top: `${mark.bbox[1] * z}px`,
        width: `${Math.max(mark.bbox[2] - mark.bbox[0], 6) * z}px`,
        height: `${Math.max(mark.bbox[3] - mark.bbox[1], 6) * z}px`,
        boxShadow: `inset 0 0 0 1.5px ${mark.color}`,
      });
      element.addEventListener('mousedown', (event) => {
        event.preventDefault();
        event.stopPropagation();
        this.handlers.removeMark?.(this, mark);
      });
      this.layer.append(element);
    }
  }

  _spanElement(line, span, index) {
    const element = document.createElement('div');
    element.className = 'span';
    if (!span.embedded) {
      element.classList.add('is-flagged');
      element.title = `La fuente «${span.font}» no está embebida; al editar se usará la más parecida.`;
    }
    element.dataset.line = line.id;
    element.dataset.span = String(index);
    // The overlay is empty — the glyphs are in the rendered image — so the text
    // is carried here, both for screen readers and so the box can be found by
    // what it covers rather than by its position on the page.
    element.dataset.text = span.text;
    element.setAttribute('role', 'textbox');
    element.setAttribute('aria-label', span.text);

    const [x0, y0, x1, y1] = span.bbox;
    const z = this.zoom;
    Object.assign(element.style, {
      left: `${x0 * z}px`,
      top: `${y0 * z}px`,
      width: `${Math.max(x1 - x0, 1) * z}px`,
      height: `${Math.max(y1 - y0, 1) * z}px`,
    });

    element.addEventListener('mousedown', (event) => {
      if (event.button !== 0) return;
      // Once this span is open for editing, a click inside it is the caret's
      // business, not ours.
      if (event.target.closest?.('.span__input')) return;
      event.stopPropagation();
      // Without this the browser moves focus to the body right after the
      // handler, undoing the focus() that puts the caret in the editable — the
      // box would open and typing would go nowhere.
      event.preventDefault();
      if (this.element.classList.contains('tool-move')) {
        this.handlers.onMoveStart?.(this, line, event);
        return;
      }
      this.handlers.onSpanActivate?.(this, line, index, element, event);
    });
    return element;
  }

  /** Draw the search hits that fall on this page. */
  setHighlights(hits, currentIndex) {
    this.layer.querySelectorAll('.hit').forEach((node) => node.remove());
    let currentNode = null;
    for (const hit of hits) {
      const element = document.createElement('div');
      element.className = hit.index === currentIndex ? 'hit hit--current' : 'hit';
      const [x0, y0, x1, y1] = hit.rect;
      const z = this.zoom;
      Object.assign(element.style, {
        left: `${x0 * z}px`,
        top: `${y0 * z}px`,
        width: `${Math.max(x1 - x0, 1) * z}px`,
        height: `${Math.max(y1 - y0, 1) * z}px`,
      });
      this.layer.append(element);
      if (hit.index === currentIndex) currentNode = element;
    }
    return currentNode;
  }

  findSpanElement(lineId, spanIndex) {
    return this.layer.querySelector(`.span[data-line="${lineId}"][data-span="${spanIndex}"]`);
  }

  findLine(lineId) {
    return this.lines.find((line) => line.id === lineId) || null;
  }

  /**
   * Sample the page background just outside `bbox`, so an editing box can hide
   * the original glyphs without painting a white patch onto coloured paper.
   */
  sampleBackground(bbox) {
    const fallback = '#ffffff';
    try {
      if (!this._sampler) {
        if (!this.image.complete || !this.image.naturalWidth) return fallback;
        const canvas = document.createElement('canvas');
        canvas.width = this.image.naturalWidth;
        canvas.height = this.image.naturalHeight;
        const context = canvas.getContext('2d', { willReadFrequently: true });
        context.drawImage(this.image, 0, 0);
        this._sampler = context;
      }
      const context = this._sampler;
      const scale = this.image.naturalWidth / this.geometry.width;
      const [x0, y0, x1, y1] = bbox;
      const probes = [
        [x0 - 2, (y0 + y1) / 2],
        [x1 + 2, (y0 + y1) / 2],
        [(x0 + x1) / 2, y0 - 2],
        [(x0 + x1) / 2, y1 + 2],
      ];
      const counts = new Map();
      for (const [px, py] of probes) {
        const x = Math.round(px * scale);
        const y = Math.round(py * scale);
        if (x < 0 || y < 0 || x >= context.canvas.width || y >= context.canvas.height) continue;
        const [r, g, b] = context.getImageData(x, y, 1, 1).data;
        const key = `rgb(${r}, ${g}, ${b})`;
        counts.set(key, (counts.get(key) || 0) + 1);
      }
      if (!counts.size) return fallback;
      // The colour seen at most probes is the paper, not a neighbouring glyph.
      return [...counts.entries()].sort((a, b) => b[1] - a[1])[0][0];
    } catch {
      return fallback;
    }
  }

  /** Build the editable element that replaces a span while it is being typed. */
  openEditor(line, spanIndex) {
    const span = line.spans[spanIndex];
    const host = this.findSpanElement(line.id, spanIndex);
    if (!host) return null;

    const input = document.createElement('div');
    input.className = 'span__input';
    input.contentEditable = 'plaintext-only';
    // On, because this is prose being written, not code. The browser's
    // underlines sit over the editing box only, never over the page itself.
    input.spellcheck = true;
    input.textContent = span.text;

    // Cover the original glyphs with the paper colour, generously enough to
    // hide ascenders and descenders that spill past the reported box.
    const z = this.zoom;
    const background = this.sampleBackground(span.bbox);
    const padding = Math.max(2, span.size * z * 0.25);
    Object.assign(input.style, {
      background,
      left: `${-padding}px`,
      top: `${(span.origin[1] - span.ascender * span.size - span.bbox[1]) * z}px`,
      paddingLeft: `${padding}px`,
      paddingRight: `${padding}px`,
      minHeight: `${(span.ascender - span.descender) * span.size * z}px`,
    });
    applyTypography(input, span, z);

    host.classList.add('is-editing');
    host.append(input);
    return input;
  }

  closeEditor(lineId, spanIndex) {
    const host = this.findSpanElement(lineId, spanIndex);
    if (!host) return;
    host.classList.remove('is-editing');
    host.querySelector('.span__input')?.remove();
  }

  /** Draw the lines the thing being dragged has just lined up with. */
  showGuides(marks) {
    this.layer.querySelectorAll('.guide').forEach((node) => node.remove());
    const z = this.zoom;
    for (const { orientation, at } of marks || []) {
      const element = document.createElement('div');
      element.className = `guide guide--${orientation}`;
      if (orientation === 'x') element.style.left = `${at * z}px`;
      else element.style.top = `${at * z}px`;
      this.layer.append(element);
    }
  }

  /** Show how far it has moved, beside the cursor. */
  showReadout(text, x, y) {
    let element = this.layer.querySelector('.readout');
    if (!element) {
      element = document.createElement('div');
      element.className = 'readout';
      this.layer.append(element);
    }
    element.textContent = text;
    element.style.left = `${x * this.zoom + 14}px`;
    element.style.top = `${y * this.zoom + 14}px`;
  }

  clearOverlays() {
    this.layer.querySelectorAll('.guide, .readout').forEach((node) => node.remove());
  }

  /** Mark the block the arrow keys would move. */
  setPicked(lines) {
    this.layer.querySelectorAll('.span.is-picked').forEach((n) => n.classList.remove('is-picked'));
    const wanted = new Set((lines || []).map((line) => line.id));
    for (const element of this.layer.querySelectorAll('.span')) {
      if (wanted.has(element.dataset.line)) element.classList.add('is-picked');
    }
  }

  /** Show where a block would land, as boxes following the cursor. */
  showGhost(rects, dx, dy) {
    this.layer.querySelectorAll('.ghost').forEach((node) => node.remove());
    const z = this.zoom;
    for (const [x0, y0, x1, y1] of rects) {
      const element = document.createElement('div');
      element.className = 'ghost';
      Object.assign(element.style, {
        left: `${(x0 + dx) * z}px`,
        top: `${(y0 + dy) * z}px`,
        width: `${Math.max(x1 - x0, 1) * z}px`,
        height: `${Math.max(y1 - y0, 1) * z}px`,
      });
      this.layer.append(element);
    }
  }

  clearGhost() {
    this.layer.querySelectorAll('.ghost').forEach((node) => node.remove());
    this.clearOverlays();
    this.element.classList.remove('is-dragging');
  }

  /** Convert a mouse event to page coordinates (points, not pixels). */
  toPagePoint(event) {
    const rect = this.element.getBoundingClientRect();
    return {
      x: (event.clientX - rect.left) / this.zoom,
      y: (event.clientY - rect.top) / this.zoom,
    };
  }

  /** Drag-a-rectangle support for the text, image and erase tools. */
  _bindMarquee() {
    this.layer.addEventListener('mousedown', (event) => {
      if (event.button !== 0) return;
      const tool = this.element.className.replace('page tool-', '');
      if (tool === 'select') {
        this.handlers.onBlankClick?.(this, event);
        return;
      }
      event.preventDefault();

      const start = this.toPagePoint(event);
      const marquee = document.createElement('div');
      marquee.className = 'marquee';
      this.layer.append(marquee);

      const anchor = this.handlers.snapPoint?.(this, start.x, start.y) || start;
      const draw = (current) => {
        const z = this.zoom;
        Object.assign(marquee.style, {
          left: `${Math.min(anchor.x, current.x) * z}px`,
          top: `${Math.min(anchor.y, current.y) * z}px`,
          width: `${Math.abs(current.x - anchor.x) * z}px`,
          height: `${Math.abs(current.y - anchor.y) * z}px`,
        });
      };

      const at = (someEvent) => {
        const point = this.toPagePoint(someEvent);
        const snapped = this.handlers.snapPoint?.(this, point.x, point.y) || point;
        this.showGuides(snapped.marks);
        return snapped;
      };

      const move = (moveEvent) => {
        const current = at(moveEvent);
        draw(current);
        this.showReadout(
          this.handlers.measure?.(
            Math.abs(current.x - anchor.x), Math.abs(current.y - anchor.y),
          ) || '',
          current.x, current.y,
        );
      };
      const up = (upEvent) => {
        document.removeEventListener('mousemove', move);
        document.removeEventListener('mouseup', up);
        marquee.remove();
        this.clearOverlays();
        const end = at(upEvent);
        const rect = [
          Math.min(anchor.x, end.x),
          Math.min(anchor.y, end.y),
          Math.max(anchor.x, end.x),
          Math.max(anchor.y, end.y),
        ];
        this.handlers.onMarquee?.(this, tool, rect, anchor);
      };

      document.addEventListener('mousemove', move);
      document.addEventListener('mouseup', up);
      draw(start);
    });
  }
}
