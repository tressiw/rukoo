// What goes wrong in this window, for the log in the profile (main's diaglog.js): errors nothing caught, and what
// app.js reports itself, such as a refresh of the list that failed. Main takes only the known kinds and fields and
// caps them; nothing here sends mail content.

// Errors already written down, so one that is reported and then rejected unhandled shows once.
const reported = new WeakSet();

// kind: 'error', 'unhandledrejection', 'refresh' or 'list'. detail: { message, stack, ms, failures, reason }.
export function report(kind, detail = {}) {
  try {
    window.mail.invoke('logRenderer', { kind, ...detail }).catch(() => {});
  } catch (_) {
    // No bridge, no log.
  }
}

// An error the caller dealt with and chose not to log: an unhandled rejection with it stays out too.
export function handled(err) {
  if (err && typeof err === 'object') reported.add(err);
}

export function reportError(kind, err, detail = {}) {
  if (err && typeof err === 'object') {
    if (reported.has(err)) return;
    reported.add(err);
  }
  const message = err && typeof err === 'object' ? String(err.message || err) : String(err);
  const stack = err && typeof err === 'object' && typeof err.stack === 'string' ? err.stack : '';
  report(kind, { ...detail, message, stack });
}

export function watchErrors() {
  window.addEventListener('error', (e) => reportError('error', e.error || e.message));
  window.addEventListener('unhandledrejection', (e) => reportError('unhandledrejection', e.reason));
}
