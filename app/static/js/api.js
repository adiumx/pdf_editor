/** Thin wrapper over the editor's HTTP API. */

/** Mirrors MAX_RENDER_ZOOM in app/main.py: the most the server will render. */
const MAX_RENDER_ZOOM = 4;

async function request(url, options = {}) {
  const response = await fetch(url, options);
  if (!response.ok) {
    let detail = `${response.status} ${response.statusText}`;
    try {
      const body = await response.json();
      if (body && body.detail) detail = typeof body.detail === 'string' ? body.detail : JSON.stringify(body.detail);
    } catch { /* response had no JSON body */ }
    throw new Error(detail);
  }
  return response;
}

async function json(url, options) {
  return (await request(url, options)).json();
}

export const api = {
  async upload(file, password) {
    const form = new FormData();
    form.append('file', file);
    if (password) form.append('password', password);
    return json('/api/documents', { method: 'POST', body: form });
  },

  async document(docId) {
    return json(`/api/documents/${docId}`);
  },

  async close(docId) {
    // Sent on unload, where a pending promise may never be awaited.
    return fetch(`/api/documents/${docId}`, { method: 'DELETE', keepalive: true });
  },

  async pageText(docId, pno) {
    return json(`/api/documents/${docId}/pages/${pno}/text`);
  },

  async fonts(docId) {
    return json(`/api/documents/${docId}/fonts`);
  },

  /**
   * The page image for a page shown at `zoom` CSS pixels per point.
   *
   * Drawn at the screen's own density rather than one image pixel per CSS
   * pixel: a phone packs about three physical pixels into each CSS pixel, so
   * an image at CSS size was stretched threefold and the page read as a blur —
   * while the line being edited, which is live text, stayed sharp. The image
   * is still laid out at CSS size; only its resolution goes up. Capped where
   * the server caps it.
   */
  renderUrl(docId, pno, zoom, revision) {
    const density = Math.max(1, window.devicePixelRatio || 1);
    const scale = Math.min(MAX_RENDER_ZOOM, Math.round(zoom * density * 100) / 100);
    return `/api/documents/${docId}/pages/${pno}/render?zoom=${scale}&revision=${revision}`;
  },

  async operations(docId, operations) {
    return json(`/api/documents/${docId}/operations`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ operations }),
    });
  },

  async inspect(docId, pno, rect) {
    return json(`/api/documents/${docId}/pages/${pno}/inspect`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ rect }),
    });
  },

  async ocr(docId, body = {}) {
    return json(`/api/documents/${docId}/ocr`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
  },

  async search(docId, body) {
    return json(`/api/documents/${docId}/search`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
  },

  async replace(docId, body) {
    return json(`/api/documents/${docId}/replace`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
  },

  async undo(docId) {
    return json(`/api/documents/${docId}/undo`, { method: 'POST' });
  },

  async redo(docId) {
    return json(`/api/documents/${docId}/redo`, { method: 'POST' });
  },

  async uploadAsset(docId, file) {
    const form = new FormData();
    form.append('file', file);
    return json(`/api/documents/${docId}/assets`, { method: 'POST', body: form });
  },

  downloadUrl(docId) {
    return `/api/documents/${docId}/download`;
  },
};
