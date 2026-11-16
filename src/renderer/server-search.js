// The search of older mail on the server under a search's results (app.js): which search a list calls for. Kept apart
// from app.js so the tests can load it.

// From this many characters, as SEARCH.min in src/main/engine.js.
const MIN = 2;

// The key of the server search for a list request, or null for none. request is what refreshOnce() asked main to list
// (scope, view, folder and query); onServer is how many messages of the folders it covers are on the server below the
// cache (olderState). The key comes from the request itself, so it always names the text that request searches for.
export function serverSearchKey(request, onServer) {
  const q = String((request && request.query) || '').trim().toLowerCase();
  if (q.length < MIN || !(onServer > 0) || request.view === 'saved') return null;
  return JSON.stringify([request.scope, request.view, request.folder || null, q]);
}

// The server search after a list request: current when it is the same search, so a search under way or done goes on;
// otherwise a new one, { key, query, state: 'searching' }, with query the request it sends; null for none. A request
// made before the user typed on gives the search of its own text, and the request after it replaces that one.
export function planServerSearch(current, request, onServer) {
  const key = serverSearchKey(request, onServer);
  if ((current ? current.key : null) === key) return current;
  return key ? { key, query: { ...request }, state: 'searching' } : null;
}
