// Renders Rukoo Voice's icons from mark.js into this folder. Run it from the repository root, in a terminal without
// ELECTRON_RUN_AS_NODE:
//
//   npx electron voice/crates/voice-app/icons/make-icons.js
//
// It draws each size into a canvas in a hidden window and writes:
//   voice.svg, voice-recording.svg        the mark as a 256 px vector, normal and recording
//   icon.ico                              the app icon: 16, 20, 24, 32, 40, 48, 64 and 256 px (exe, installer, shortcuts)
//   icon.icns                             the macOS app icon: the mark on Apple's rounded square, 16 to 1024 px
//   32x32.png, 128x128.png, 128x128@2x.png, icon.png (512 px)   the PNG set Tauri lists for other platforms
//   tray-recording.ico, tray-recording.png   the tray icon while a meeting records (build.rs embeds the .ico)
//   installer-sidebar.bmp                 the picture on the installer's first and last page (164 x 314)
'use strict';

const { app, BrowserWindow } = require('electron');
const fs = require('fs');
const path = require('path');
const { markSvg, macIconSvg } = require('./mark.js');

const HERE = __dirname;
const APP_SIZES = [16, 20, 24, 32, 40, 48, 64, 256];
const TRAY_SIZES = [16, 20, 24, 32, 40, 48, 64];
// The pixel sizes a macOS app icon holds. 32, 64, 128, 256 and 512 px each serve two types, once as they are and
// once as the 2x bitmap of the size below, so there are eleven entries for seven images.
const ICNS_SIZES = [16, 32, 64, 128, 256, 512, 1024];
const ICNS_TYPES = [
  ['icp4', 16],
  ['icp5', 32],
  ['icp6', 64],
  ['ic07', 128],
  ['ic08', 256],
  ['ic09', 512],
  ['ic10', 1024],
  ['ic11', 32], // 16 px at 2x
  ['ic12', 64], // 32 px at 2x
  ['ic13', 256], // 128 px at 2x
  ['ic14', 512], // 256 px at 2x
];
// Rukoo's dark canvas (--bg in src/renderer/styles.css), the background the dark-head pigeon was drawn for.
const SIDEBAR_BG = '#141519';

// Draws SVG text into a canvas; returns the PNG and the straight (not premultiplied) RGBA pixels.
async function draw(win, svg, width, height, { background = null, mark = null } = {}) {
  const src = `data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}`;
  const result = await win.webContents.executeJavaScript(`(async () => {
    const img = new Image();
    img.src = ${JSON.stringify(src)};
    await img.decode();
    const canvas = document.createElement('canvas');
    canvas.width = ${width};
    canvas.height = ${height};
    const g = canvas.getContext('2d');
    const background = ${JSON.stringify(background)};
    const mark = ${JSON.stringify(mark)};
    if (background) { g.fillStyle = background; g.fillRect(0, 0, ${width}, ${height}); }
    if (mark) g.drawImage(img, mark.x, mark.y, mark.size, mark.size);
    else g.drawImage(img, 0, 0, ${width}, ${height});
    const pixels = g.getImageData(0, 0, ${width}, ${height}).data;
    let binary = '';
    for (let i = 0; i < pixels.length; i += 0x8000) binary += String.fromCharCode.apply(null, pixels.subarray(i, i + 0x8000));
    return { png: canvas.toDataURL('image/png').split(',')[1], rgba: btoa(binary) };
  })()`);
  return { png: Buffer.from(result.png, 'base64'), rgba: Buffer.from(result.rgba, 'base64') };
}

// One ICO image as a 32-bit DIB: bottom-up BGRA rows, then the 1-bit AND mask (set where fully transparent).
function dib(size, rgba) {
  const maskRow = Math.ceil(size / 32) * 4;
  const pixels = size * size * 4;
  const out = Buffer.alloc(40 + pixels + maskRow * size);
  out.writeUInt32LE(40, 0);
  out.writeInt32LE(size, 4);
  out.writeInt32LE(size * 2, 8); // colour rows plus mask rows
  out.writeUInt16LE(1, 12);
  out.writeUInt16LE(32, 14);
  out.writeUInt32LE(pixels + maskRow * size, 20);
  for (let y = 0; y < size; y++) {
    const row = size - 1 - y;
    for (let x = 0; x < size; x++) {
      const s = (row * size + x) * 4;
      const d = 40 + (y * size + x) * 4;
      out[d] = rgba[s + 2];
      out[d + 1] = rgba[s + 1];
      out[d + 2] = rgba[s];
      out[d + 3] = rgba[s + 3];
      if (rgba[s + 3] === 0) out[40 + pixels + y * maskRow + (x >> 3)] |= 0x80 >> (x & 7);
    }
  }
  return out;
}

// An ICO file. Images below 256 px are DIBs, which every Windows component reads; 256 px is PNG, as Windows expects.
function ico(images) {
  const header = Buffer.alloc(6 + 16 * images.length);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  let offset = header.length;
  const data = images.map(({ size, png, rgba }, i) => {
    const bytes = size >= 256 ? png : dib(size, rgba);
    const entry = 6 + 16 * i;
    header.writeUInt8(size >= 256 ? 0 : size, entry);
    header.writeUInt8(size >= 256 ? 0 : size, entry + 1);
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(bytes.length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += bytes.length;
    return bytes;
  });
  return Buffer.concat([header, ...data]);
}

// An ICNS file: an 8-byte header, then one 8-byte header and the PNG for each type. macOS reads PNG in every one of
// these types, so the PNGs the window renders are the data; no iconutil or other platform tool is involved.
function icns(images) {
  const entries = ICNS_TYPES.map(([type, size]) => {
    const png = images.get(size);
    if (!png) throw new Error(`no ${size} px image for ${type}`);
    const head = Buffer.alloc(8);
    head.write(type, 0, 'latin1');
    head.writeUInt32BE(png.length + 8, 4);
    return Buffer.concat([head, png]);
  });
  const header = Buffer.alloc(8);
  header.write('icns', 0, 'latin1');
  header.writeUInt32BE(8 + entries.reduce((total, entry) => total + entry.length, 0), 4);
  return Buffer.concat([header, ...entries]);
}

// Reads an ICNS back and checks the container and every PNG's own header, so a wrong size fails here rather than in
// Finder: the magic, the total length, each entry inside the file, and PNGs of the size its type names.
function checkIcns(file, types) {
  const buf = fs.readFileSync(file);
  if (buf.subarray(0, 4).toString('latin1') !== 'icns') throw new Error(`${file} doesn't start with icns`);
  if (buf.readUInt32BE(4) !== buf.length) throw new Error(`${file} says it is ${buf.readUInt32BE(4)} bytes, not ${buf.length}`);
  const found = [];
  let offset = 8;
  while (offset < buf.length) {
    if (offset + 8 > buf.length) throw new Error(`${file}: an entry starts past the end`);
    const type = buf.subarray(offset, offset + 4).toString('latin1');
    const length = buf.readUInt32BE(offset + 4);
    if (length < 8 || offset + length > buf.length) throw new Error(`${file}: ${type} runs past the end`);
    const png = buf.subarray(offset + 8, offset + length);
    if (png.subarray(1, 4).toString('latin1') !== 'PNG') throw new Error(`${file}: ${type} is not a PNG`);
    const wanted = types.find(([name]) => name === type);
    if (!wanted) throw new Error(`${file}: has an unexpected ${type} entry`);
    const width = png.readUInt32BE(16);
    const height = png.readUInt32BE(20);
    if (width !== wanted[1] || height !== wanted[1]) {
      throw new Error(`${file}: ${type} is ${width} x ${height} instead of ${wanted[1]}`);
    }
    found.push(type);
    offset += length;
  }
  if (found.join() !== types.map(([type]) => type).join()) {
    throw new Error(`${file}: has ${found.join()} instead of ${types.map(([type]) => type).join()}`);
  }
}

// A 24-bit BMP, the format NSIS wants for its wizard pictures.
function bmp(width, height, rgba) {
  const row = Math.ceil((width * 3) / 4) * 4;
  const out = Buffer.alloc(54 + row * height);
  out.write('BM', 0, 'latin1');
  out.writeUInt32LE(out.length, 2);
  out.writeUInt32LE(54, 10);
  out.writeUInt32LE(40, 14);
  out.writeInt32LE(width, 18);
  out.writeInt32LE(height, 22);
  out.writeUInt16LE(1, 26);
  out.writeUInt16LE(24, 28);
  out.writeUInt32LE(row * height, 34);
  for (let y = 0; y < height; y++) {
    const src = (height - 1 - y) * width * 4;
    for (let x = 0; x < width; x++) {
      const s = src + x * 4;
      const d = 54 + y * row + x * 3;
      out[d] = rgba[s + 2];
      out[d + 1] = rgba[s + 1];
      out[d + 2] = rgba[s];
    }
  }
  return out;
}

// Reads an ICO back and checks it lists exactly the sizes we wrote, so a broken writer fails here, not in rc.exe.
function checkIco(file, sizes) {
  const buf = fs.readFileSync(file);
  const count = buf.readUInt16LE(4);
  const found = [];
  for (let i = 0; i < count; i++) {
    const e = 6 + 16 * i;
    const size = buf[e] || 256;
    const length = buf.readUInt32LE(e + 8);
    const offset = buf.readUInt32LE(e + 12);
    if (offset + length > buf.length) throw new Error(`${file}: image ${size} runs past the end`);
    const png = buf.subarray(offset, offset + 4).toString('latin1') === '\x89PNG';
    if (!png && buf.readInt32LE(offset + 4) !== size) throw new Error(`${file}: image ${size} has the wrong width`);
    found.push(size);
  }
  if (found.join() !== sizes.join()) throw new Error(`${file}: has ${found.join()} instead of ${sizes.join()}`);
}

app.disableHardwareAcceleration();
// Keep Electron's own profile out of %APPDATA%.
app.setPath('userData', path.join(require('os').tmpdir(), 'rukoo-voice-make-icons'));
app.whenReady().then(async () => {
  let code = 0;
  try {
    const win = new BrowserWindow({ show: false, width: 64, height: 64 });
    await win.loadURL('data:text/html,<html><body></body></html>');
    const write = (name, bytes) => fs.writeFileSync(path.join(HERE, name), bytes);

    write('voice.svg', markSvg(256));
    write('voice-recording.svg', markSvg(256, { recording: true }));

    const appImages = [];
    for (const size of APP_SIZES) appImages.push({ size, ...(await draw(win, markSvg(size), size, size)) });
    write('icon.ico', ico(appImages));
    checkIco(path.join(HERE, 'icon.ico'), APP_SIZES);

    // The macOS icon is its own composition: Apple's rounded square with the mark inside it. The PNGs for its sizes
    // are drawn only for the .icns; the platform's flat PNG set above stays the bare mark.
    const macImages = new Map();
    for (const size of ICNS_SIZES) macImages.set(size, (await draw(win, macIconSvg(size), size, size)).png);
    write('icon.icns', icns(macImages));
    checkIcns(path.join(HERE, 'icon.icns'), ICNS_TYPES);

    for (const [name, size] of [['32x32.png', 32], ['128x128.png', 128], ['128x128@2x.png', 256], ['icon.png', 512]]) {
      write(name, (await draw(win, markSvg(size), size, size)).png);
    }

    const trayImages = [];
    for (const size of TRAY_SIZES) {
      trayImages.push({ size, ...(await draw(win, markSvg(size, { recording: true }), size, size)) });
    }
    write('tray-recording.ico', ico(trayImages));
    checkIco(path.join(HERE, 'tray-recording.ico'), TRAY_SIZES);
    write('tray-recording.png', trayImages.find((i) => i.size === 32).png);

    // The pigeon is cut off at its chest, so it stands on the sidebar's bottom edge, as on a taskbar.
    const sidebar = await draw(win, markSvg(148), 164, 314, { background: SIDEBAR_BG, mark: { x: 8, y: 170, size: 148 } });
    write('installer-sidebar.bmp', bmp(164, 314, sidebar.rgba));
    console.log(`wrote Rukoo Voice's icons to ${HERE}`);
  } catch (e) {
    console.error(e);
    code = 1;
  }
  app.exit(code);
});
