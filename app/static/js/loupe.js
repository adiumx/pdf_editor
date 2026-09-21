/**
 * A magnified view of the text being edited, floated above the finger.
 *
 * Putting a caret between two particular letters is the one thing a finger is
 * worst at: the fingertip covers exactly the letters it is aiming between, and
 * a PDF's body text on a phone is a few pixels tall. So the text is redrawn
 * larger, above the hand, with the caret marked — the same answer phones
 * themselves reached.
 *
 * What is magnified is the *editing box*, not the rendered page underneath it:
 * the page shows the words as they were before this edit, and a magnifier that
 * disagreed with what is being typed would be worse than none.
 */

const MAGNIFICATION = 2.4;
const WIDTH = 190;
const HEIGHT = 46;

// How far above the finger the bubble floats. Enough to clear a fingertip,
// which is the whole point of it.
const LIFT = 30;

/** Distance in pixels from the start of `input`'s text to the caret. */
function caretOffset(input) {
  const selection = window.getSelection();
  if (!selection?.rangeCount) return 0;
  const caret = selection.getRangeAt(0);
  if (!input.contains(caret.startContainer)) return 0;
  const upToCaret = document.createRange();
  upToCaret.selectNodeContents(input);
  try {
    upToCaret.setEnd(caret.startContainer, caret.startOffset);
  } catch {
    return 0;  // the selection moved on between reading it and measuring it
  }
  return upToCaret.getBoundingClientRect().width;
}

export class Loupe {
  constructor() {
    this.element = null;
    this.text = null;
    this.caret = null;
  }

  get isOpen() {
    return this.element !== null;
  }

  _build() {
    this.element = document.createElement('div');
    this.element.className = 'loupe';
    this.element.style.width = `${WIDTH}px`;
    this.element.style.height = `${HEIGHT}px`;

    this.text = document.createElement('div');
    this.text.className = 'loupe__text';

    this.caret = document.createElement('div');
    this.caret.className = 'loupe__caret';
    this.caret.style.left = `${WIDTH / 2}px`;

    this.element.append(this.text, this.caret);
    document.body.append(this.element);
  }

  /**
   * Show the bubble over a point, reading the text and the caret out of the
   * box being edited.
   */
  show(input, clientX, clientY) {
    if (!input) return;
    if (!this.element) this._build();

    const styles = getComputedStyle(input);
    Object.assign(this.text.style, {
      fontFamily: styles.fontFamily,
      fontWeight: styles.fontWeight,
      fontStyle: styles.fontStyle,
      fontSize: `${parseFloat(styles.fontSize) * MAGNIFICATION}px`,
      color: styles.color,
    });
    this.element.style.background = styles.backgroundColor;
    this.text.textContent = input.textContent;

    // Slide the text so the caret lands on the bubble's middle, where the
    // mark is: what the bubble is for is saying which two letters the caret
    // is between, so that is what sits in the centre of it.
    this.text.style.left = `${WIDTH / 2 - caretOffset(input) * MAGNIFICATION}px`;

    // Kept on screen: at the edges of a narrow phone the bubble would
    // otherwise hang off the side, which is where the first and last letters
    // of a line are — exactly the ones worth magnifying.
    const left = Math.min(Math.max(clientX - WIDTH / 2, 4), window.innerWidth - WIDTH - 4);
    this.element.style.left = `${left}px`;
    this.element.style.top = `${Math.max(4, clientY - HEIGHT - LIFT)}px`;
  }

  hide() {
    this.element?.remove();
    this.element = null;
    this.text = null;
    this.caret = null;
  }
}
