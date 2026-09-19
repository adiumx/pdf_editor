/** Thin wrapper over the editor's HTTP API. */

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

  renderUrl(docId, pno, zoom, revision) {
    return `/api/documents/${docId}/pages/${pno}/render?zoom=${zoom}&revision=${revision}`;
  },

  async operations(docId, operations) {
    return json(`/api/documents/${docId}/operations`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ operations }),
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
