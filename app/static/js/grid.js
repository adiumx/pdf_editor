/**
 * The grid, and everything that helps put things exactly where they belong.
 *
 * Two kinds of help, and they answer different questions. The grid imposes a
 * rhythm the document never had: useful for placing something new. The guides
 * are read off the page itself — the margin its text already starts at, the
 * edges of the lines around where you are dropping — and answer the question
 * people actually ask, which is "line this up with that".
 */

const MM = 2.834645669;  // points per millimetre
const INCH = 72;

// How near a target has to be, in pixels on screen, before it pulls. Measured
// in screen space on purpose: at a deeper zoom you are working finer, and the
// pull should follow the eye rather than the page's own units.
const PULL = 7;

// Beyond this many guide candidates the page is too dense for them to mean
// anything, and finding the nearest would start to cost.
const MAX_GUIDES = 400;

export const STEPS = [
  { label: '1 mm', points: MM },
  { label: '2 mm', points: 2 * MM },
  { label: '5 mm', points: 5 * MM },
  { label: '10 mm', points: 10 * MM },
  { label: '5 pt', points: 5 },
  { label: '10 pt', points: 10 },
  { label: '20 pt', points: 20 },
  { label: '1/8 pulg.', points: INCH / 8 },
  { label: '1/4 pulg.', points: INCH / 4 },
];

export class Grid {
  constructor(elements, editor) {
    this.el = elements;
    this.editor = editor;
    this.visible = false;
    this.snapToGrid = true;
    this.snapToGuides = true;
    this.spacing = 5 * MM;
    this._guides = new WeakMap();
    this._bind();
  }

  get isOpen() {
    return !this.el.bar.hidden;
  }

  toggleBar() {
    this.el.bar.hidden = !this.el.bar.hidden;
  }

  _bind() {
    for (const { label, points } of STEPS) {
      const option = document.createElement('option');
      option.value = String(points);
      option.textContent = label;
      this.el.step.append(option);
    }
    this.el.step.value = String(this.spacing);

    this.el.show.addEventListener('change', () => {
      this.visible = this.el.show.checked;
      this.paintAll();
    });
    this.el.step.addEventListener('change', () => {
      this.spacing = Number(this.el.step.value) || this.spacing;
      this.paintAll();
    });
    this.el.snapGrid.addEventListener('change', () => {
      this.snapToGrid = this.el.snapGrid.checked;
    });
    this.el.snapGuides.addEventListener('change', () => {
      this.snapToGuides = this.el.snapGuides.checked;
    });
    this.el.close.addEventListener('click', () => {
      this.el.bar.hidden = true;
    });
  }

  /* ---------- drawing ---------- */

  paintAll() {
    for (const view of this.editor.pages.values()) this.paint(view);
  }

  /** Lay the grid over one page, ruled at the current step. */
  paint(view) {
    const layer = view.layer;
    if (!this.visible) {
      layer.style.backgroundImage = '';
      return;
    }
    const step = this.spacing * view.zoom;
    const major = step * 5;
    const thin = 'rgba(47, 109, 246, .15)';
    const thick = 'rgba(47, 109, 246, .3)';
    layer.style.backgroundImage = [
      `repeating-linear-gradient(to right, ${thick} 0 1px, transparent 1px ${major}px)`,
      `repeating-linear-gradient(to bottom, ${thick} 0 1px, transparent 1px ${major}px)`,
      `repeating-linear-gradient(to right, ${thin} 0 1px, transparent 1px ${step}px)`,
      `repeating-linear-gradient(to bottom, ${thin} 0 1px, transparent 1px ${step}px)`,
    ].join(', ');
  }

  /* ---------- snapping ---------- */

  /** The positions on a page worth lining up with, from the page itself. */
  candidates(view, exclude = []) {
    const skip = new Set(exclude.map((line) => line.id));
    let cached = this._guides.get(view);
    if (!cached || cached.lines !== view.lines) {
      const xs = new Set();
      const ys = new Set();
      for (const line of view.lines) {
        xs.add(round(line.bbox[0]));
        xs.add(round(line.bbox[2]));
        if (line.measure) xs.add(round(line.measure));
        ys.add(round(line.bbox[1]));
        ys.add(round(line.bbox[3]));
        if (xs.size + ys.size > MAX_GUIDES) break;
      }
      cached = { lines: view.lines, xs: [...xs], ys: [...ys] };
      this._guides.set(view, cached);
    }
    if (!skip.size) return cached;
    // A block never lines up with itself.
    const own = { xs: new Set(), ys: new Set() };
    for (const line of view.lines) {
      if (!skip.has(line.id)) continue;
      own.xs.add(round(line.bbox[0]));
      own.xs.add(round(line.bbox[2]));
      own.ys.add(round(line.bbox[1]));
      own.ys.add(round(line.bbox[3]));
    }
    return {
      lines: cached.lines,
      xs: cached.xs.filter((v) => !own.xs.has(v)),
      ys: cached.ys.filter((v) => !own.ys.has(v)),
    };
  }

  /** The nearest thing to line up with, or null when nothing is near. */
  _pull(value, targets, tolerance) {
    let best = null;
    let distance = tolerance;
    for (const target of targets) {
      const gap = Math.abs(target - value);
      if (gap <= distance) {
        distance = gap;
        best = target;
      }
    }
    return best;
  }

  _gridNear(value, tolerance) {
    if (!this.snapToGrid || this.spacing <= 0) return null;
    const nearest = Math.round(value / this.spacing) * this.spacing;
    return Math.abs(nearest - value) <= tolerance ? nearest : null;
  }

  /**
   * Adjust a drag so the block lands on something.
   *
   * Every edge of the block is a candidate to be lined up, and the one that
   * needs the smallest nudge wins — which is what makes it feel like the edge
   * you were aiming with is the one that caught.
   */
  snapDelta(view, box, dx, dy, { constrain = false, exclude = [] } = {}) {
    if (constrain) {
      if (Math.abs(dx) > Math.abs(dy)) dy = 0;
      else dx = 0;
    }
    const tolerance = PULL / view.zoom;
    const guides = this.snapToGuides ? this.candidates(view, exclude) : { xs: [], ys: [] };
    const marks = [];

    const axis = (offset, edges, targets, frozen) => {
      if (frozen) return offset;
      let best = offset;
      let gap = tolerance;
      for (const edge of edges) {
        const moved = edge + offset;
        for (const target of [this._gridNear(moved, tolerance), this._pull(moved, targets, tolerance)]) {
          if (target === null) continue;
          const shift = target - moved;
          if (Math.abs(shift) < gap) {
            gap = Math.abs(shift);
            best = offset + shift;
          }
        }
      }
      return best;
    };

    const snappedX = axis(dx, [box[0], (box[0] + box[2]) / 2, box[2]], guides.xs, constrain && dx === 0);
    const snappedY = axis(dy, [box[1], (box[1] + box[3]) / 2, box[3]], guides.ys, constrain && dy === 0);

    for (const [edge, targets, offset, orientation] of [
      [box[0], guides.xs, snappedX, 'x'],
      [box[2], guides.xs, snappedX, 'x'],
      [box[1], guides.ys, snappedY, 'y'],
      [box[3], guides.ys, snappedY, 'y'],
    ]) {
      const at = edge + offset;
      if (targets.some((target) => Math.abs(target - at) < 0.05)) marks.push({ orientation, at });
    }
    return { dx: snappedX, dy: snappedY, marks };
  }

  /** The same, for a rectangle being drawn rather than moved. */
  snapPoint(view, x, y) {
    const tolerance = PULL / view.zoom;
    const guides = this.snapToGuides ? this.candidates(view) : { xs: [], ys: [] };
    const pick = (value, targets) => {
      const options = [this._gridNear(value, tolerance), this._pull(value, targets, tolerance)]
        .filter((option) => option !== null);
      if (!options.length) return { value, snapped: false };
      const best = options.reduce((a, b) => (Math.abs(a - value) <= Math.abs(b - value) ? a : b));
      return { value: best, snapped: true };
    };
    const px = pick(x, guides.xs);
    const py = pick(y, guides.ys);
    const marks = [];
    if (px.snapped) marks.push({ orientation: 'x', at: px.value });
    if (py.snapped) marks.push({ orientation: 'y', at: py.value });
    return { x: px.value, y: py.value, marks };
  }

  /** How far something moved, in whatever unit the step is set in. */
  format(dx, dy) {
    const label = this.el.step.selectedOptions[0]?.textContent || '';
    if (label.includes('mm')) return `${(dx / MM).toFixed(1)} × ${(dy / MM).toFixed(1)} mm`;
    if (label.includes('pulg')) return `${(dx / INCH).toFixed(2)} × ${(dy / INCH).toFixed(2)} pulg.`;
    return `${dx.toFixed(1)} × ${dy.toFixed(1)} pt`;
  }
}

function round(value) {
  return Math.round(value * 10) / 10;
}
