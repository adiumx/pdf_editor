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
      this.handlers.onSpanActivate?.(this, line, index, element, event);
    });
    return element;
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
    input.spellcheck = false;
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

      const draw = (current) => {
        const z = this.zoom;
        Object.assign(marquee.style, {
          left: `${Math.min(start.x, current.x) * z}px`,
          top: `${Math.min(start.y, current.y) * z}px`,
          width: `${Math.abs(current.x - start.x) * z}px`,
          height: `${Math.abs(current.y - start.y) * z}px`,
        });
      };

      const move = (moveEvent) => draw(this.toPagePoint(moveEvent));
      const up = (upEvent) => {
        document.removeEventListener('mousemove', move);
        document.removeEventListener('mouseup', up);
        marquee.remove();
        const end = this.toPagePoint(upEvent);
        const rect = [
          Math.min(start.x, end.x),
          Math.min(start.y, end.y),
          Math.max(start.x, end.x),
          Math.max(start.y, end.y),
        ];
        this.handlers.onMarquee?.(this, tool, rect, start);
      };

      document.addEventListener('mousemove', move);
      document.addEventListener('mouseup', up);
      draw(start);
    });
  }
}
