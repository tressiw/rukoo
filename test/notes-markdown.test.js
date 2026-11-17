'use strict';

// How the meeting notes reader reads a line of Markdown (src/renderer/notes-markdown.js). The round trip through the
// editor needs a DOM and is in test/e2e/meeting.spec.js.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');

// A browser module in a CommonJS package: Node takes it as ESM from a data: URL. The sanitizer it imports needs a
// DOM, so these tests read the HTML the reader hands it; what the reader makes is escaped already.
async function load() {
  const source = fs.readFileSync(path.join(__dirname, '../src/renderer/notes-markdown.js'), 'utf8').replace(/^import .*;\r?\n/gm, '');
  return import(`data:text/javascript,${encodeURIComponent(`const sanitizeAgentHtml = (html) => html;\n${source}`)}`);
}
const line = async (text) => (await load()).markdownToNotes(text);

test('a backtick or a backslash in a link address is part of the address', async () => {
  // A lone backtick in the address used to open a code span that closed in the code after it.
  assert.equal(await line('See [t](<https://a.example/`x>) and `npm start` after'), '<div>See <a href="https://a.example/`x">t</a> and <code>npm start</code> after</div>');
  // A backslash at the end of the address used to escape the > that closes it.
  assert.equal(await line('[t](<https://a.example/x\\>) and \\*'), '<div><a href="https://a.example/x\\">t</a> and *</div>');
  assert.equal(await line('[t](<https://a.example/q?a=``1``&b=\\_>)'), '<div><a href="https://a.example/q?a=``1``&amp;b=\\_">t</a></div>');
});

test('escapes, code spans and links are read from left to right, so the one that starts first wins', async () => {
  assert.equal(await line('`[x](<https://a.example/>)` and [y](<https://b.example/`z`>)'), '<div><code>[x](&lt;https://a.example/&gt;)</code> and <a href="https://b.example/`z`">y</a></div>');
  assert.equal(await line('`[x](<https://a.example/`\\>) after'), '<div><code>[x](&lt;https://a.example/</code>&gt;) after</div>');
  assert.equal(await line('\\[x](<https://a.example/>)'), '<div>[x](&lt;https://a.example/&gt;)</div>');
  // The text of a link keeps its escapes, its code and its formatting; a ] in code doesn't end it.
  assert.equal(await line('[a\\]b `c]d` **e**](<https://a.example/>)'), '<div><a href="https://a.example/">a]b <code>c]d</code> <b>e</b></a></div>');
  assert.equal(await line('[outer [inner](<https://a.example/>)'), '<div><a href="https://a.example/">outer [inner</a></div>');
  assert.equal(await line('<u>[x](<https://a.example/>) y</u>'), '<div><u><a href="https://a.example/">x</a> y</u></div>');
});

test('a code span closes with a run of as many backticks', async () => {
  assert.equal(await line('a `` ` `` b'), '<div>a <code>`</code> b</div>');
  assert.equal(await line('`a``b`'), '<div><code>a``b</code></div>');
  assert.equal(await line('`a\\`'), '<div><code>a\\</code></div>');
  assert.equal(await line('``a` and `b'), '<div>``a<code>and</code>b</div>');
  assert.equal(await line('`  `'), '<div><code>  </code></div>', 'code of spaces only keeps them');
});

test('an address without angle brackets ends at a space or a ), and takes escapes', async () => {
  assert.equal(await line('[t](https://a.example/x\\)y) after'), '<div><a href="https://a.example/x)y">t</a> after</div>');
  assert.equal(await line('[t](https://a.example/ x)'), '<div>[t](https://a.example/ x)</div>');
  // An address main doesn't open leaves the text without a link.
  assert.equal(await line('[u](javascript:alert(1)) [m](mailto:a@b.example)'), '<div>u) <a href="mailto:a@b.example">m</a></div>');
  assert.equal(await line('[a](<b>) [](<https://a.example/>)'), '<div>a [](&lt;https://a.example/&gt;)</div>');
});

// The timing guards: before the reader went from left to right, 100,000 [ took more than six seconds.
const fast = (fn, label) => {
  const start = process.hrtime.bigint();
  fn();
  const ms = Number(process.hrtime.bigint() - start) / 1e6;
  assert.ok(ms < 500, `${label} took ${Math.round(ms)} ms`);
};

test('a long line of brackets, backticks or tags that never close is read in linear time', async () => {
  const { markdownToNotes } = await load();
  const runs = ['[', '[a](<', '[a](', '[a](<b', '[a](<`', '`', '``a', '<u>', '\\', '](<', '[a\\', '`[a](<'];
  for (const run of runs) {
    const text = run.repeat(Math.ceil(100000 / run.length)).slice(0, 100000);
    fast(() => markdownToNotes(text), JSON.stringify(run));
  }
  // Every opening run of a length short of its closing one.
  const ticks = Array.from({ length: 440 }, (_, n) => `${'`'.repeat(n + 1)}a`).join('');
  fast(() => markdownToNotes(ticks), 'backtick runs of every length');
});
