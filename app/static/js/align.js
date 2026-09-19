/**
 * Lining blocks up, and spacing them out.
 *
 * Kept apart from the editor because it is pure geometry: boxes in, offsets
 * out. Nothing here touches the page, which is what makes it possible to check
 * the arithmetic without a document open.
 */

/** The edge each alignment meets, read off the boxes themselves. */
const EDGES = {
  left: { axis: 0, pick: (values) => Math.min(...values), at: (box) => box[0] },
  right: { axis: 0, pick: (values) => Math.max(...values), at: (box) => box[2] },
  top: { axis: 1, pick: (values) => Math.min(...values), at: (box) => box[1] },
  bottom: { axis: 1, pick: (values) => Math.max(...values), at: (box) => box[3] },
};

const CENTRES = {
  'center-h': { axis: 0, at: (box) => (box[0] + box[2]) / 2 },
  'center-v': { axis: 1, at: (box) => (box[1] + box[3]) / 2 },
};

/** Every alignment this module understands, in the order the bar shows them. */
export const ALIGNMENTS = [
  'left', 'center-h', 'right',
  'top', 'center-v', 'bottom',
  'spread-h', 'spread-v',
];

/** How many blocks each one needs before it means anything. */
export function needs(how) {
  return how.startsWith('spread-') ? 3 : 2;
}

/**
 * The (dx, dy) each box has to move by, in the order they were given.
 *
 * Returns null when the request cannot be honoured — too few boxes for what
 * was asked, or a name this does not know.
 */
export function alignmentDeltas(how, boxes) {
  if (!boxes || boxes.length < needs(how)) return null;
  const flat = boxes.map(() => [0, 0]);

  const edge = EDGES[how];
  if (edge) {
    const target = edge.pick(boxes.map(edge.at));
    boxes.forEach((box, index) => { flat[index][edge.axis] = target - edge.at(box); });
    return flat;
  }

  const centre = CENTRES[how];
  if (centre) {
    // The middle of what is selected, not the mean of the middles: a wide
    // block among narrow ones would otherwise drag the line towards itself.
    const axis = centre.axis;
    const low = Math.min(...boxes.map((box) => box[axis]));
    const high = Math.max(...boxes.map((box) => box[axis + 2]));
    const target = (low + high) / 2;
    boxes.forEach((box, index) => { flat[index][axis] = target - centre.at(box); });
    return flat;
  }

  if (how === 'spread-h' || how === 'spread-v') {
    const axis = how === 'spread-h' ? 0 : 1;
    return spread(boxes, axis, flat);
  }
  return null;
}

/**
 * Share the gaps evenly, leaving the outermost two blocks alone.
 *
 * The space to share is what is left over once every block's own length is
 * taken out, so blocks of different sizes end up with equal *gaps* rather than
 * equal spacing between their edges — which is what the eye reads as even.
 */
function spread(boxes, axis, flat) {
  const order = boxes
    .map((box, index) => ({ index, start: box[axis], end: box[axis + 2] }))
    .sort((a, b) => a.start - b.start);

  const first = order[0];
  const last = order[order.length - 1];
  const span = last.end - first.start;
  const occupied = order.reduce((total, item) => total + (item.end - item.start), 0);
  const gap = (span - occupied) / (order.length - 1);

  let cursor = first.start;
  for (const item of order) {
    flat[item.index][axis] = cursor - item.start;
    cursor += (item.end - item.start) + gap;
  }
  return flat;
}
