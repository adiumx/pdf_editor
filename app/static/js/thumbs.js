/** The page rail: a thumbnail per page, drag to reorder, rotate and delete. */

import { api } from './api.js';

export class Thumbnails {
  constructor(list, editor) {
    this.list = list;
    this.editor = editor;
    this.dragFrom = null;
    this.signature = null;
  }

  /** What the rail depends on. Anything else changing must not rebuild it. */
  _signature() {
    const { doc, revision } = this.editor;
    if (!doc) return null;
    return [doc.id, revision, doc.pages.map((p) => `${p.width}x${p.height}@${p.rotation}`).join('|')].join('#');
  }

  render() {
    // Rebuilding the rail re-downloads every thumbnail, so it happens only when
    // the pages themselves changed — not on every editor state change.
    const signature = this._signature();
    if (signature === this.signature) return;
    this.signature = signature;

    const { doc, revision } = this.editor;
    this.list.replaceChildren();
    if (!doc) return;

    doc.pages.forEach((geometry, index) => {
      const item = document.createElement('li');
      const button = document.createElement('button');
      button.className = 'thumb';
      button.draggable = true;
      button.dataset.page = String(index);
      button.title = `Ir a la página ${index + 1}`;

      const image = document.createElement('img');
      image.src = api.renderUrl(doc.id, index, 0.22, revision);
      image.alt = '';
      // Reserve the right shape up front so the rail does not jump about as
      // thumbnails arrive, and rotation is reflected before the image loads.
      const turned = geometry.rotation % 180 !== 0;
      const width = turned ? geometry.height : geometry.width;
      const height = turned ? geometry.width : geometry.height;
      image.style.aspectRatio = `${width} / ${height}`;

      const label = document.createElement('span');
      label.className = 'thumb__label';
      label.textContent = String(index + 1);

      const actions = document.createElement('span');
      actions.className = 'thumb__actions';
      actions.append(
        this._action('↻', 'Girar 90°', () =>
          this.editor.pageOperation({
            op: 'rotate_page',
            page: index,
            rotation: (geometry.rotation + 90) % 360,
          }),
        ),
        this._action('🗑', 'Borrar esta página', () => {
          if (doc.page_count <= 1) return;
          if (!confirm(`¿Borrar la página ${index + 1}?`)) return;
          this.editor.pageOperation({ op: 'delete_page', page: index });
        }),
      );

      button.append(image, label, actions);
      button.addEventListener('click', () => {
        this.editor.pages.get(index)?.element.scrollIntoView({ behavior: 'smooth', block: 'start' });
      });

      button.addEventListener('dragstart', (event) => {
        this.dragFrom = index;
        button.classList.add('is-dragging');
        event.dataTransfer.effectAllowed = 'move';
        event.dataTransfer.setData('text/plain', String(index));
      });
      button.addEventListener('dragend', () => {
        button.classList.remove('is-dragging');
        this.dragFrom = null;
      });
      button.addEventListener('dragover', (event) => {
        if (this.dragFrom === null || this.dragFrom === index) return;
        event.preventDefault();
        button.classList.add('is-dragover');
      });
      button.addEventListener('dragleave', () => button.classList.remove('is-dragover'));
      button.addEventListener('drop', (event) => {
        event.preventDefault();
        button.classList.remove('is-dragover');
        const from = this.dragFrom;
        this.dragFrom = null;
        if (from === null || from === index) return;
        // move_page inserts *before* `to`, so moving down needs one more.
        const to = from < index ? index + 1 : index;
        this.editor.pageOperation({ op: 'move_page', page: from, to });
      });

      item.append(button);
      this.list.append(item);
    });
  }

  _action(label, title, onClick) {
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = label;
    button.title = title;
    button.addEventListener('click', (event) => {
      event.stopPropagation();
      onClick();
    });
    return button;
  }
}
