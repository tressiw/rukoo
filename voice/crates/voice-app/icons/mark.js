// The Rukoo Voice mark: Rukoo's pigeon with the pill's five-bar waveform in front of its beak.
// markSvg() returns the mark as SVG text for one pixel size, with the bars on whole pixels, so every icon size stays
// sharp. macIconSvg() puts the same mark on the rounded square macOS wants for an app icon. make-icons.js rasterizes
// both; voice.svg and voice-recording.svg are the mark's 256 px output, kept as the vector source.
'use strict';

const fs = require('fs');
const path = require('path');

// The pigeon comes from Rukoo itself, with the lighter head Rukoo's own app icon uses (scripts/make-icon.js),
// so the two marks stay one family when the pigeon changes.
const PIGEON_SVG = path.join(__dirname, '..', '..', '..', '..', 'src', 'renderer', 'assets', 'rukoo-icon-dark.svg');
// Where the pigeon's shapes sit in its 268-unit box.
const PIGEON = { left: 33, bottom: 268 };

// The pill's bars rest at these fractions of their height (voice/ui/pill.css), so the mark draws the same wave.
const WAVE = [0.4, 0.75, 1, 0.65, 0.45];
// The magenta of the pigeon's neck: it holds up on light and dark taskbars alike.
const BAR = '#bd2dc4';
// The recording dot. Rukoo marks a recording meeting with a red dot too (styles.css .rec-dot, --danger).
const RECORDING = '#e5383b';

// The large layout, in units of a 256 px icon. Larger and smaller sizes scale it and round the bars to pixels.
const LARGE = {
  pigeon: { scale: 0.8, left: 6, bottom: 250 },
  bars: { width: 14, gap: 7, height: 112, middle: 92, right: 250 },
  dot: { x: 206, y: 206, r: 44, gap: 12 },
};

// Below 32 px the five bars would blur into one stroke, so these sizes draw three, laid out by hand in pixels:
// [x, y, width, height] per bar. The pigeon's height is in pixels too.
const SMALL = {
  16: { pigeon: 14, bars: [[11, 3, 1, 5], [13, 2, 1, 7], [15, 4, 1, 3]], dot: { x: 12.5, y: 12.5, r: 3.5, gap: 1 } },
  20: { pigeon: 17, bars: [[12, 4, 2, 6], [15, 2, 2, 9], [18, 4, 2, 5]], dot: { x: 15.5, y: 15.5, r: 4.5, gap: 1 } },
  24: { pigeon: 20, bars: [[16, 5, 2, 7], [19, 3, 2, 11], [22, 6, 2, 5]], dot: { x: 18.5, y: 18.5, r: 5.5, gap: 1.5 } },
};

function pigeonShapes() {
  const svg = fs.readFileSync(PIGEON_SVG, 'utf8');
  const match = svg.match(/<g id="pigeon">([\s\S]*?)<\/g>/);
  if (!match) throw new Error(`no <g id="pigeon"> in ${PIGEON_SVG}`);
  return match[1].trim();
}

const round = (n) => Math.round(n * 100) / 100;

function rect([x, y, w, h], fill) {
  // A bar one or two pixels wide keeps square ends; rounding them would only blur the end pixels.
  const rx = w > 2 ? w / 2 : 0;
  return `<rect x="${round(x)}" y="${round(y)}" width="${round(w)}" height="${round(h)}" rx="${round(rx)}" fill="${fill}"/>`;
}

// The layout for one pixel size: the pigeon's transform, the bars as pixel rectangles and the recording dot.
function layout(size) {
  if (SMALL[size]) {
    const small = SMALL[size];
    const scale = small.pigeon / PIGEON.bottom;
    return {
      pigeon: { scale, x: -PIGEON.left * scale, y: size - small.pigeon },
      bars: small.bars,
      dot: small.dot,
    };
  }
  const k = size / 256;
  const p = LARGE.pigeon;
  const b = LARGE.bars;
  const scale = p.scale * k;
  // Whole pixels for the bars: widths and gaps at least one pixel, heights even so each bar centres on the same row.
  const width = Math.max(1, Math.round(b.width * k));
  const gap = Math.max(1, Math.round(b.gap * k));
  const total = WAVE.length * width + (WAVE.length - 1) * gap;
  const right = Math.round(b.right * k);
  const middle = Math.round(b.middle * k);
  const bars = WAVE.map((f, i) => {
    const h = Math.max(width, 2 * Math.round((b.height * k * f) / 2));
    return [right - total + i * (width + gap), middle - h / 2, width, h];
  });
  const d = LARGE.dot;
  return {
    pigeon: { scale, x: p.left * k - PIGEON.left * scale, y: p.bottom * k - PIGEON.bottom * scale },
    bars,
    dot: { x: d.x * k, y: d.y * k, r: d.r * k, gap: d.gap * k },
  };
}

// The mark at `size` pixels. `recording` adds the red dot, cut out of the pigeon and the bars.
function markSvg(size, { recording = false } = {}) {
  const l = layout(size);
  const p = l.pigeon;
  let body =
    `<g transform="translate(${round(p.x)} ${round(p.y)}) scale(${p.scale})">${pigeonShapes()}</g>` +
    l.bars.map((bar) => rect(bar, BAR)).join('');
  let defs = '';
  if (recording) {
    const d = l.dot;
    defs =
      `<defs><mask id="cut"><rect width="${size}" height="${size}" fill="#fff"/>` +
      `<circle cx="${round(d.x)}" cy="${round(d.y)}" r="${round(d.r + d.gap)}" fill="#000"/></mask></defs>`;
    body = `<g mask="url(#cut)">${body}</g><circle cx="${round(d.x)}" cy="${round(d.y)}" r="${round(d.r)}" fill="${RECORDING}"/>`;
  }
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}" width="${size}" height="${size}" ` +
    `role="img" aria-label="Rukoo Voice">${defs}${body}</svg>\n`
  );
}

// A macOS app icon, in Apple's current shape: a rounded square with a margin, on Rukoo's dark canvas (--bg in
// src/renderer/styles.css, the background the dark-head pigeon was drawn for). macOS shows an app's icon flat and
// unmasked, so the shape, the margin and the canvas are part of the picture rather than something the system adds.
// The template's proportions: an 824-unit tile on a 1024-unit canvas, 100 units of margin, a 185.4-unit corner radius.
const MAC = {
  canvas: 1024,
  tile: 824,
  radius: 185.4,
  background: '#141519',
  // The height of the mark's drawn content as a fraction of the tile, which leaves room around it and keeps the
  // pigeon and the bars off the rounded corners.
  height: 0.72,
};

// The box the mark's shapes occupy inside its 256-unit square, which is not the square itself: the pigeon stands on
// the bottom edge of the layout and leaves a strip under its chest empty, and the bars reach further right than the
// pigeon. Centring this box, rather than the square, spaces the colours evenly in the tile.
const MARK_CONTENT = (() => {
  const l = layout(256);
  const p = l.pigeon;
  return {
    left: p.x + PIGEON.left * p.scale,
    top: p.y,
    right: Math.max(...l.bars.map(([x, , width]) => x + width)),
    bottom: p.y + PIGEON.bottom * p.scale,
  };
})();

// The app icon at `size` pixels for one side. The mark is laid out for the pixel size it gets in the tile, so its
// bars stay on whole pixels at every size, and its content box is centred in the tile.
function macIconSvg(size) {
  const k = size / MAC.canvas;
  const tile = MAC.tile * k;
  const margin = (size - tile) / 2;
  const contentHeight = MARK_CONTENT.bottom - MARK_CONTENT.top;
  const markSize = Math.max(1, Math.round(((MAC.height * tile) / contentHeight) * 256));
  const centre = (a, b) => (markSize * (a + b)) / 512;
  const x = size / 2 - centre(MARK_CONTENT.left, MARK_CONTENT.right);
  const y = size / 2 - centre(MARK_CONTENT.top, MARK_CONTENT.bottom);
  const inner = markSvg(markSize).replace(/^<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '');
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${size} ${size}" width="${size}" height="${size}" ` +
    `role="img" aria-label="Rukoo Voice">` +
    `<rect x="${round(margin)}" y="${round(margin)}" width="${round(tile)}" height="${round(tile)}" ` +
    `rx="${round(MAC.radius * k)}" fill="${MAC.background}"/>` +
    `<g transform="translate(${round(x)} ${round(y)})">${inner}</g></svg>\n`
  );
}

module.exports = { markSvg, macIconSvg, layout, SMALL };
