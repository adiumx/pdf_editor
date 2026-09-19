/**
 * The find bar: searching the document, stepping through results, replacing.
 *
 * Matches come from the server, which is the only side that knows how the text
 * is really laid out. What is kept here is just which one is current and where
 * they are on the page, so they can be drawn and scrolled to.
 */

import { api } from './api.js';
import { toast, withBusy } from './ui.js';

const DEBOUNCE = 220;

export class SearchBar {
  constructor(elements, editor) {
    this.el = elements;
    this.editor = editor;
    this.matches = [];
    this.current = -1;
    this.matchCase = false;
    this.wholeWord = false;
    this.timer = null;
    this.searchedRevision = null;
    this._bind();
  }

  get isOpen() {
    return !this.el.bar.hidden;
  }

  open() {
    this.el.bar.hidden = false;
    this.el.query.focus();
    this.el.query.select();
    if (this.el.query.value) this.run();
  }

  close() {
    this.el.bar.hidden = true;
    this.matches = [];
    this.current = -1;
    this.draw();
  }

  toggle() {
    if (this.isOpen) this.close();
    else this.open();
  }

  _bind() {
    this.el.query.addEventListener('input', () => this.schedule());
    this.el.query.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        this.step(event.shiftKey ? -1 : 1);
      } else if (event.key === 'Escape') {
        event.preventDefault();
        this.close();
      }
    });
    this.el.next.addEventListener('click', () => this.step(1));
    this.el.previous.addEventListener('click', () => this.step(-1));
    this.el.close.addEventListener('click', () => this.close());

    this.el.matchCase.addEventListener('click', () => {
      this.matchCase = !this.matchCase;
      this.el.matchCase.classList.toggle('is-active', this.matchCase);
      this.run();
    });
    this.el.wholeWord.addEventListener('click', () => {
      this.wholeWord = !this.wholeWord;
      this.el.wholeWord.classList.toggle('is-active', this.wholeWord);
      this.run();
    });

    this.el.replaceAll.addEventListener('click', () => this.replaceAll());
    this.el.replacement.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        this.replaceAll();
      } else if (event.key === 'Escape') {
        this.close();
      }
    });
  }

  schedule() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.run(), DEBOUNCE);
  }

  /** Ask the server for every occurrence of what is typed. */
  async run() {
    if (!this.editor.isOpen) return;
    const query = this.el.query.value;
    if (!query) {
      this.matches = [];
      this.current = -1;
      this.report();
      this.draw();
      return;
    }
    try {
      const result = await api.search(this.editor.doc.id, {
        query,
        match_case: this.matchCase,
        whole_word: this.wholeWord,
      });
      this.matches = result.matches;
      this.searchedRevision = this.editor.revision;
      this.current = this.matches.length ? 0 : -1;
      this.report();
      this.draw();
      if (this.current >= 0) this.reveal();
    } catch (error) {
      toast(String(error.message || error), 'error');
    }
  }

  /** Re-run after an edit, because every position may have moved. */
  refresh() {
    if (this.isOpen && this.el.query.value && this.editor.revision !== this.searchedRevision) {
      this.run();
    }
  }

  step(direction) {
    if (!this.matches.length) return;
    this.current = (this.current + direction + this.matches.length) % this.matches.length;
    this.report();
    this.draw();
    this.reveal();
  }

  report() {
    const { count } = this.el;
    if (!this.el.query.value) {
      count.textContent = '';
      count.classList.remove('find-count--none');
      return;
    }
    if (!this.matches.length) {
      count.textContent = 'sin resultados';
      count.classList.add('find-count--none');
      return;
    }
    count.classList.remove('find-count--none');
    count.textContent = `${this.current + 1} de ${this.matches.length}`;
  }

  draw() {
    for (const [pno, view] of this.editor.pages) {
      const hits = [];
      this.matches.forEach((match, index) => {
        if (match.page === pno) hits.push({ rect: match.rect, index });
      });
      view.setHighlights(hits, this.current);
    }
  }

  reveal() {
    const match = this.matches[this.current];
    if (!match) return;
    const view = this.editor.pages.get(match.page);
    const node = view?.layer.querySelector('.hit--current');
    (node || view?.element)?.scrollIntoView({ behavior: 'smooth', block: 'center' });
  }

  async replaceAll() {
    if (!this.editor.isOpen) return;
    const query = this.el.query.value;
    if (!query) return;
    const replacement = this.el.replacement.value;
    if (this.matches.length > 1 &&
        !confirm(`¿Reemplazar ${this.matches.length} apariciones de «${query}»?`)) {
      return;
    }
    try {
      const result = await withBusy('Reemplazando…', () =>
        api.replace(this.editor.doc.id, {
          query,
          replacement,
          match_case: this.matchCase,
          whole_word: this.wholeWord,
        }),
      );
      await this.editor.afterExternalEdit(result);
      toast(
        result.replaced
          ? `${result.replaced} ${result.replaced === 1 ? 'reemplazo' : 'reemplazos'}.`
          : 'No se encontró nada que reemplazar.',
        result.replaced ? 'info' : 'warn',
      );
      await this.run();
    } catch (error) {
      toast(String(error.message || error), 'error');
    }
  }
}
