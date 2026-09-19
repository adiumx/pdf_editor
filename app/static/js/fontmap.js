/**
 * Guessing a browser font for a PDF font.
 *
 * This only drives the live preview while a span is being typed into: the
 * authoritative appearance comes from the server re-rendering the page with the
 * PDF's own font program. The closer the guess, the less the text shifts when
 * the edit lands.
 */

const SUBSET_PREFIX = /^[A-Z]{6}\+/;
const STYLE_SUFFIX = /[-,_ ]?(regular|bold|italic|oblique|black|heavy|light|medium|semibold|demibold|extrabold|book|roman|condensed|narrow|mt|ps|std|pro)+$/i;

/** Split "ABCDEF+LiberationSerif-Italic" into a usable family name. */
export function familyOf(fontName) {
  let name = (fontName || '').replace(SUBSET_PREFIX, '');
  name = name.split(',')[0];
  let previous;
  do {
    previous = name;
    name = name.replace(STYLE_SUFFIX, '');
  } while (name !== previous && name.length > 2);
  return name.trim() || 'sans-serif';
}

/** Insert spaces into CamelCase names so "LiberationSerif" also matches. */
function spaced(name) {
  return name.replace(/([a-z0-9])([A-Z])/g, '$1 $2');
}

const GENERIC = {
  sans: '"Helvetica Neue", Helvetica, Arial, "Liberation Sans", sans-serif',
  serif: 'Georgia, "Times New Roman", "Liberation Serif", serif',
  mono: '"SF Mono", Menlo, Consolas, "Liberation Mono", monospace',
};

/** A CSS font-family stack that tries the document's font first. */
export function cssFontStack(span) {
  const family = familyOf(span.font);
  const generic = span.mono ? GENERIC.mono : span.serif ? GENERIC.serif : GENERIC.sans;
  if (!span.font || ['sans', 'serif', 'mono'].includes(span.font)) return generic;
  const variants = new Set([family, spaced(family)]);
  const quoted = [...variants].map((name) => `"${name}"`).join(', ');
  return `${quoted}, ${generic}`;
}

/** Apply a span's typography to an element, scaled for the current zoom. */
export function applyTypography(element, span, zoom) {
  element.style.fontFamily = cssFontStack(span);
  element.style.fontSize = `${span.size * zoom}px`;
  element.style.fontWeight = span.bold ? '700' : '400';
  element.style.fontStyle = span.italic ? 'italic' : 'normal';
  element.style.color = span.color;
}
