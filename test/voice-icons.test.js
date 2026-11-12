'use strict';

// Rukoo Voice's macOS app icon (voice/crates/voice-app/icons/icon.icns). make-icons.js writes the container itself,
// from the PNGs it renders, so that no iconutil or other Mac tool is needed; this reads it back the way macOS does,
// which fails here instead of on a Mac when a size is missing or a PNG doesn't match its type.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');

const ICNS = path.join(__dirname, '..', 'voice', 'crates', 'voice-app', 'icons', 'icon.icns');
// Every type a macOS app icon carries, in the order make-icons.js writes them: 1x, plus 2x where Apple's types
// define one (32 px serves 32 at 1x and 16 at 2x, and so on). The small sizes are PNGs under the icp types.
const TYPES = [
  ['icp4', 16],
  ['icp5', 32],
  ['icp6', 64],
  ['ic07', 128],
  ['ic08', 256],
  ['ic09', 512],
  ['ic10', 1024],
  ['ic11', 32],
  ['ic12', 64],
  ['ic13', 256],
  ['ic14', 512],
];

// The entries of an ICNS file: a type and the bytes after its 8-byte header. Throws where the container is broken.
function entries(buf) {
  assert.equal(buf.subarray(0, 4).toString('latin1'), 'icns', 'the file does not start with icns');
  assert.equal(buf.readUInt32BE(4), buf.length, 'the header length is not the file length');
  const found = [];
  for (let offset = 8; offset < buf.length;) {
    assert.ok(offset + 8 <= buf.length, `an entry at ${offset} runs past the end`);
    const type = buf.subarray(offset, offset + 4).toString('latin1');
    const length = buf.readUInt32BE(offset + 4);
    assert.ok(length >= 8 && offset + length <= buf.length, `${type} runs past the end`);
    found.push({ type, body: buf.subarray(offset + 8, offset + length) });
    offset += length;
  }
  return found;
}

test('icon.icns holds every size macOS uses, and each PNG is that size', () => {
  const found = entries(fs.readFileSync(ICNS));
  assert.deepEqual(found.map((entry) => entry.type), TYPES.map(([type]) => type));
  for (const { type, body } of found) {
    const size = TYPES.find(([name]) => name === type)[1];
    assert.equal(body.subarray(1, 4).toString('latin1'), 'PNG', `${type} is not a PNG`);
    assert.equal(body.readUInt32BE(16), size, `${type} is not ${size} px wide`);
    assert.equal(body.readUInt32BE(20), size, `${type} is not ${size} px high`);
  }
  const sizes = [...new Set(found.map(({ type }) => TYPES.find(([name]) => name === type)[1]))];
  assert.deepEqual(sizes.sort((a, b) => a - b), [16, 32, 64, 128, 256, 512, 1024]);
});
