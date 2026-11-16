'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');

// A browser module in a CommonJS package: Node takes it as ESM from a data: URL.
async function load() {
  const source = fs.readFileSync(path.join(__dirname, '../src/renderer/server-search.js'), 'utf8');
  return import(`data:text/javascript,${encodeURIComponent(source)}`);
}

const request = (query, extra = {}) => ({ scope: 'acc', view: 'inbox', folder: null, query, ...extra });

// #112: the search of older mail on the server under a search's results.
test('a list request calls for the server search of its own text, from two characters and with older mail to find', async () => {
  const { serverSearchKey } = await load();
  assert.equal(serverSearchKey(request(' Parcel '), 3), JSON.stringify(['acc', 'inbox', null, 'parcel']));
  assert.equal(serverSearchKey(request('p'), 3), null);
  assert.equal(serverSearchKey(request('parcel'), 0), null, 'the folders it covers are all in the cache');
  assert.equal(serverSearchKey(request('parcel', { view: 'saved' }), 3), null);
  assert.notEqual(serverSearchKey(request('parcel', { view: 'everything' }), 3), serverSearchKey(request('parcel'), 3));
});

test('a request made before the user typed on searches its own text, and the next request starts the search of the new text', async () => {
  const { planServerSearch } = await load();
  // refreshOnce() asked main to list "parcel"; the box says "thursday" by the time main answers.
  const stale = planServerSearch(null, request('parcel'), 5);
  assert.equal(stale.query.query, 'parcel');
  assert.equal(stale.key, JSON.stringify(['acc', 'inbox', null, 'parcel']));
  // The refresh after it, for "thursday", replaces that search instead of taking it for its own.
  const fresh = planServerSearch(stale, request('thursday'), 5);
  assert.notEqual(fresh, stale);
  assert.deepEqual(fresh, { key: JSON.stringify(['acc', 'inbox', null, 'thursday']), query: request('thursday'), state: 'searching' });
  // The same search again leaves a search under way, or done, as it is.
  const done = { ...fresh, state: 'done', more: 0 };
  assert.equal(planServerSearch(done, request('Thursday '), 5), done);
  assert.equal(planServerSearch(done, request(''), 5), null);
  assert.equal(planServerSearch(null, request(''), 5), null);
});
