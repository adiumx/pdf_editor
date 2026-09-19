/** Toasts, the busy overlay, and the small bits of chrome around them. */

const toasts = document.getElementById('toasts');
const busy = document.getElementById('busy');
const busyLabel = document.getElementById('busy-label');
const status = document.getElementById('status');

let busyDepth = 0;

export function toast(message, kind = 'info', timeout = 5200) {
  const element = document.createElement('div');
  element.className = `toast toast--${kind}`;
  element.textContent = message;
  toasts.append(element);
  setTimeout(() => {
    element.style.opacity = '0';
    element.style.transition = 'opacity .2s';
    setTimeout(() => element.remove(), 220);
  }, timeout);
  return element;
}

/** Run `task` with the busy overlay up; nested calls keep it up until the last. */
export async function withBusy(label, task) {
  busyDepth += 1;
  busyLabel.textContent = label;
  busy.hidden = false;
  try {
    return await task();
  } finally {
    busyDepth -= 1;
    if (busyDepth <= 0) {
      busyDepth = 0;
      busy.hidden = true;
    }
  }
}

export function setStatus(text) {
  status.textContent = text || '';
}

/** Report warnings the server attached to an applied edit. */
export function reportWarnings(warnings) {
  const seen = new Set();
  for (const warning of warnings || []) {
    if (seen.has(warning.message)) continue;
    seen.add(warning.message);
    toast(warning.message, 'warn', 7000);
  }
}
