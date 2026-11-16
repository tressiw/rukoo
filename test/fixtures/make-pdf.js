'use strict';

// Builds small PDFs for tests: pages of text lines, in Helvetica (WinAnsi) or in a Type0 font whose ToUnicode map
// gives the text, with compressed content and, optionally, the fonts and page tree in an object stream.
const zlib = require('zlib');
const iconv = require('iconv-lite');

// Helvetica with WinAnsiEncoding: the text in Windows-1252 bytes.
const literal = (text) => `(${iconv.encode(text, 'win1252').toString('latin1').replace(/[\\()]/g, (c) => `\\${c}`)})`;

// pages: [[line, ...], ...]. type0: the text goes through a ToUnicode map, as in PDFs from Word or Chrome. cmap: that map's
// text instead of the one made for the text.
function makePdf(pages, { compress = true, type0 = false, objectStream = false, cmap: ownCmap = null } = {}) {
  const chars = [...new Set(pages.flat().join(''))];
  const cid = (ch) => (chars.indexOf(ch) + 1).toString(16).padStart(4, '0');
  const show = (line) => (type0 ? `<${[...line].map(cid).join('')}> Tj` : `${literal(line)} Tj`);
  const objects = [];
  const add = (body) => objects.push(body) && objects.length;
  const stream = (dict, data) => {
    const bytes = compress ? zlib.deflateSync(Buffer.from(data, 'latin1')) : Buffer.from(data, 'latin1');
    return { dict: `${dict}${compress ? '/Filter/FlateDecode' : ''}/Length ${bytes.length}`, bytes };
  };

  const catalog = add('<</Type/Catalog/Pages 2 0 R>>');
  const tree = add(null);
  let font;
  if (type0) {
    const map = chars.map((ch) => `<${cid(ch)}> <${Buffer.from(ch, 'utf16le').swap16().toString('hex')}>`).join('\n');
    const cmap = ownCmap || `/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n${chars.length} beginbfchar\n${map}\nendbfchar\nendcmap end end`;
    const toUnicode = add(stream('<<', cmap));
    const cidFont = add(`<</Type/Font/Subtype/CIDFontType2/BaseFont/Fixture/CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>/DW 600>>`);
    font = add(`<</Type/Font/Subtype/Type0/BaseFont/Fixture/Encoding/Identity-H/DescendantFonts[${cidFont} 0 R]/ToUnicode ${toUnicode} 0 R>>`);
  } else {
    font = add('<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>');
  }
  const kids = [];
  for (const lines of pages) {
    const content = `BT /F1 12 Tf 72 760 Td 16 TL ${lines.map((l, i) => (i ? `T* ${show(l)}` : show(l))).join(' ')} ET`;
    const contents = add(stream('<<', content));
    kids.push(add(`<</Type/Page/Parent ${tree} 0 R/MediaBox[0 0 612 792]/Resources<</Font<</F1 ${font} 0 R>>>>/Contents ${contents} 0 R>>`));
  }
  objects[tree - 1] = `<</Type/Pages/Kids[${kids.map((k) => `${k} 0 R`).join(' ')}]/Count ${kids.length}>>`;

  // The page tree and the font move into an object stream, as PDF 1.5 writers do.
  const packed = objectStream ? [tree, font] : [];
  if (packed.length) {
    let head = '';
    let body = '';
    for (const num of packed) {
      head += `${num} ${body.length} `;
      body += `${objects[num - 1]}\n`;
    }
    add(stream(`<</Type/ObjStm/N ${packed.length}/First ${head.length}`, head + body));
  }

  const parts = [Buffer.from('%PDF-1.5\n%\xe2\xe3\xcf\xd3\n', 'latin1')];
  const offsets = [];
  let size = parts[0].length;
  objects.forEach((body, i) => {
    const num = i + 1;
    if (packed.includes(num)) return;
    offsets[num] = size;
    const chunk =
      typeof body === 'string'
        ? Buffer.from(`${num} 0 obj\n${body}\nendobj\n`, 'latin1')
        : Buffer.concat([Buffer.from(`${num} 0 obj\n${body.dict}>>\nstream\n`, 'latin1'), body.bytes, Buffer.from('\nendstream\nendobj\n', 'latin1')]);
    parts.push(chunk);
    size += chunk.length;
  });
  const xref = [`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`];
  for (let n = 1; n <= objects.length; n++) xref.push(offsets[n] ? `${String(offsets[n]).padStart(10, '0')} 00000 n \n` : '0000000000 65535 f \n');
  parts.push(Buffer.from(`${xref.join('')}trailer\n<</Size ${objects.length + 1}/Root ${catalog} 0 R>>\nstartxref\n${size}\n%%EOF\n`, 'latin1'));
  return Buffer.concat(parts);
}

// One page with compressed content and Helvetica as F1. fonts: the page's fonts; resources: its other resources;
// objects: more objects, numbered from 10; contents: the object with its content, or the Contents value itself.
function onePage(content, { fonts = '/F1 5 0 R', resources = '', objects = '', contents = 4 } = {}) {
  const body = zlib.deflateSync(Buffer.from(content, 'latin1'));
  const head =
    '%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n' +
    `3 0 obj<</Type/Page/Parent 2 0 R/Resources<</Font<<${fonts}>>${resources}>>/Contents ${typeof contents === 'number' ? `${contents} 0 R` : contents}>>endobj\n` +
    `5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>endobj\n${objects}` +
    `4 0 obj<</Length ${body.length}/Filter/FlateDecode>>stream\n`;
  return Buffer.concat([Buffer.from(head, 'latin1'), body, Buffer.from('\nendstream\nendobj\ntrailer<</Root 1 0 R>>\n%%EOF\n', 'latin1')]);
}

// One page whose content is object 10, stored as written: no filter unless stream adds one. stream: more entries
// of its dictionary; length: its Length, the content's own by default; before and after: more objects, written
// before and after it.
function plainPage(content, { stream = '', length = content.length, before = '', after = '' } = {}) {
  return onePage('', { contents: 10, objects: `${before}10 0 obj<<${stream}/Length ${length}>>stream\n${content}\nendstream\nendobj\n${after}` });
}

// An object with stream data, compressed when deflate is set.
function streamObject(num, dict, data, deflate = false) {
  const bytes = deflate ? zlib.deflateSync(Buffer.from(data, 'latin1')).toString('latin1') : data;
  return `${num} 0 obj<<${dict}${deflate ? '/Filter/FlateDecode' : ''}/Length ${bytes.length}>>stream\n${bytes}\nendstream\nendobj\n`;
}

const cmapOf = (body) => `/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n${body}\nendcmap end end`;
// A ToUnicode map in which one range fills every two-byte code.
const FULL_RANGE = cmapOf('1 beginbfrange <0000> <FFFF> <0041> endbfrange');
const TYPE0 = '/Type/Font/Subtype/Type0/Encoding/Identity-H';

// PDFs built to hang the reader or run it out of memory, each far below the 10 MB attachment cap. The first three a
// review found: a ToUnicode range past the safe integers, a long run of digits on which the object scan
// backtracked, and one compressed string of four million glyphs. The others are the same kinds of work and
// memory growing faster than the file, elsewhere in the reader.
function hostilePdfs() {
  let fontObjects = '';
  let fontNames = '/F1 5 0 R';
  let fontShows = '';
  for (let i = 0; i < 2000; i++) {
    fontObjects += `${100 + i} 0 obj<<${TYPE0}/ToUnicode ${5000 + i} 0 R>>endobj\n${streamObject(5000 + i, '', FULL_RANGE)}`;
    fontNames += `/T${i} ${100 + i} 0 R`;
    fontShows += `/T${i} 12 Tf <0001> Tj `;
  }
  let packedHead = '';
  for (let i = 0; i < 100000; i++) packedHead += `${10 + i} 0 `;
  const sameOffsets = streamObject(9, `/Type/ObjStm/N 100000/First ${packedHead.length}`, `${packedHead}[${'1 '.repeat(200000)}`, true);
  const manyHead = Array.from({ length: 600000 }, (_, i) => `${10 + i} ${2 * i}`).join(' ');
  const manyPacked = streamObject(9, `/Type/ObjStm/N 600000/First ${manyHead.length + 1}`, `${manyHead} ${'0 '.repeat(600000)}`, true);
  return {
    cmapRange: makePdf([['Hi']], { type0: true, cmap: cmapOf('1 beginbfrange <20000000000000> <20000000000000> <0041> endbfrange') }),
    digits: Buffer.concat([Buffer.from('%PDF-1.5\n'), Buffer.alloc(500000, '1')]),
    hugeString: makePdf([['A'.repeat(4000000)]]),
    // Finding objects: headers inside an array that never closes, trailers that never close, a hex string that
    // never closes in each object, and an object stream whose objects all start at offset 0, before an array that
    // never closes.
    nestedObjects: Buffer.from(`%PDF-1.5\n${'1 0 obj ['.repeat(100000)}`, 'latin1'),
    openTrailers: Buffer.from(`%PDF-1.5\n${'trailer ['.repeat(100000)}`, 'latin1'),
    openHex: Buffer.from(`%PDF-1.5\n${'1 0 obj < endobj\n'.repeat(200000)}`, 'latin1'),
    // A string or comment that endobj cuts runs on to its own end: strings that never close, strings that each
    // close only at the end of the file, and comments with no line break after them, in every object. Each run on
    // looked through the rest of the file.
    openStrings: Buffer.from(`%PDF-1.5\n${'1 0 obj ( endobj\n'.repeat(200000)}`, 'latin1'),
    farStrings: Buffer.from(`%PDF-1.5\n${'1 0 obj(endobj\n'.repeat(200000)}${')'.repeat(200000)}`, 'latin1'),
    openComments: Buffer.from(`%PDF-1.5\n${'1 0 obj<</A % endobj '.repeat(200000)}`, 'latin1'),
    sameOffsets: Buffer.from(`%PDF-1.5\n${sameOffsets}trailer<</Root 10 0 R>>\n`, 'latin1'),
    // A second review: an object stream whose First is past the safe integers, which the lexer took as its end and
    // read characters past the string for, and one that claims six hundred thousand objects in 1.6 MB.
    objStmFirst: Buffer.from('%PDF-1.5\n9 0 obj<</Type/ObjStm/N 2/First 36028797018963968/Length 5>>stream\n10 0 \nendstream endobj\ntrailer<</Root 10 0 R>>\n', 'latin1'),
    manyPacked: Buffer.from(`%PDF-1.5\n${manyPacked}trailer<</Root 10 0 R>>\n`, 'latin1'),
    // A page of a thousand and one: past the page cap, which said nothing was missing.
    pages1001: makePdf(Array.from({ length: 1001 }, (_, i) => [`Page ${i + 1}`])),
    // A raw deflate stream (no zlib header) of 100 MiB as a page's content a thousand times: its overflow came back
    // as an empty stream, so each one inflated the bomb again. And one unfiltered megabyte listed three hundred
    // times, which was copied for each before the content limit counted it.
    rawBomb: onePage('', { contents: `[${'10 0 R '.repeat(1000)}]`, objects: streamObject(10, '/Filter/FlateDecode', zlib.deflateRawSync(Buffer.alloc(100 * 1024 * 1024)).toString('latin1')) }),
    repeatedStream: onePage('', { contents: `[${'10 0 R '.repeat(300)}]`, objects: streamObject(10, '', ' '.repeat(1024 * 1024)) }),
    // ToUnicode maps: a range list that never closes, one full range a hundred thousand times, blocks that never
    // end, two thousand fonts of one full range each, and a font in the resources themselves, set again and again.
    cmapList: makePdf([['Hi']], { type0: true, cmap: cmapOf(`1 beginbfrange\n${'<0000> <0001> ['.repeat(200000)}\nendbfrange`) }),
    cmapRepeat: makePdf([['Hi']], { type0: true, cmap: cmapOf(`100000 beginbfrange\n${'<0000> <FFFF> <0041>\n'.repeat(100000)}endbfrange`) }),
    cmapOpen: makePdf([['Hi']], { type0: true, cmap: 'beginbfrange '.repeat(200000) }),
    manyFonts: onePage(`BT ${fontShows}ET`, { fonts: fontNames, objects: fontObjects }),
    directFont: onePage(`BT ${'/D 12 Tf '.repeat(300000)}ET`, { fonts: `/F1 5 0 R/D<<${TYPE0}/ToUnicode 10 0 R>>`, objects: streamObject(10, '', FULL_RANGE) }),
    // CID widths for a range past the safe integers.
    widths: Buffer.from(makePdf([['Hi']], { type0: true }).toString('latin1').replace('/DW 600', '/DW 600/W[36028797018963968 36028797018963970 500]'), 'latin1'),
    // Content: a page of forty thousand lines, a TJ of three million strings, a string of four million escapes, a
    // form of a megabyte drawn three million times, and ASCII85 zeros that come out four times their inflated size.
    lines: makePdf([Array(40000).fill('word word word word word')]),
    bigTJ: onePage(`BT /F1 12 Tf [${'(a)'.repeat(3000000)}] TJ ET`),
    escapes: onePage(`BT /F1 12 Tf (${'\\101'.repeat(4000000)}) Tj ET`),
    formAgain: onePage('/X Do '.repeat(3000000), { resources: '/XObject<</X 10 0 R>>', objects: streamObject(10, '/Type/XObject/Subtype/Form', `%${'x'.repeat(1000000)}\n`) }),
    zeros: onePage('BT ET', { contents: 10, objects: streamObject(10, '/Filter[/FlateDecode/ASCII85Decode]', zlib.deflateSync(Buffer.from(`${'z'.repeat(16000000)}~>`)).toString('latin1')) })
  };
}

// Text, then more operators than the reader runs: it stops there, with "start" and the rest of the page unread.
const manyOpsPdf = () => onePage(`BT /F1 12 Tf 72 700 Td (start) Tj ET ${'q Q '.repeat(3000001)}BT /F1 12 Tf 72 680 Td (never read) Tj ET`);

const START = 'BT /F1 12 Tf 72 720 Td (start) Tj ET ';
const line = (text, y = 700) => `BT /F1 12 Tf 72 ${y} Td (${text}) Tj ET `;

// Forms drawn inside each other, n deep; the innermost one shows "deepest".
function nestedForms(n) {
  let objects = '';
  for (let i = 1; i <= n; i++) {
    const inner = i < n ? `/X${i + 1} Do` : line('deepest', 600);
    const resources = `/Resources<</Font<</F1 5 0 R>>${i < n ? `/XObject<</X${i + 1} ${10 + i} 0 R>>` : ''}>>`;
    objects += streamObject(9 + i, `/Type/XObject/Subtype/Form${resources}`, inner);
  }
  return onePage(`${START}/X1 Do`, { resources: '/XObject<</X1 10 0 R>>', objects });
}

// A page tree with one page at the top and another n levels down, which shows "deep".
function deepTree(n) {
  let objects = '%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R 100 0 R]/Count 2>>endobj\n';
  objects += '3 0 obj<</Type/Page/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>endobj\n5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>endobj\n';
  for (let i = 0; i < n; i++) objects += `${100 + i} 0 obj<</Type/Pages/Kids[${101 + i} 0 R]/Count 1>>endobj\n`;
  objects += `${100 + n} 0 obj<</Type/Page/Resources<</Font<</F1 5 0 R>>>>/Contents 6 0 R>>endobj\n`;
  objects += streamObject(4, '', START) + streamObject(6, '', line('deep'));
  return Buffer.from(`${objects}trailer<</Root 1 0 R>>\n`, 'latin1');
}

// The font reached through a chain of n references.
function refChain(n) {
  let objects = '';
  for (let i = 0; i < n; i++) objects += `${20 + i} 0 obj ${21 + i} 0 R endobj\n`;
  objects += `${20 + n} 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>endobj\n`;
  return onePage(`${START}BT /F2 12 Tf 72 700 Td (lost) Tj ET`, { fonts: '/F1 5 0 R/F2 20 0 R', objects });
}

// n objects whose title never closes, near the start of a file that white space at the end of the page makes
// long, so each one looks through almost all of it.
const openStrings = (n) => plainPage(`${START}${' '.repeat(10000)}`, { before: '11 0 obj<</Title( endobj\n'.repeat(n) });

// The same n titles, then font 5 again with "t" as "x" and two strings that endobj cuts: the first runs on to its
// end, the second only 20000 bytes on, with what the n titles left. Short of that, the font is read without them.
const spentStrings = (n) =>
  plainPage(START, {
    before: '11 0 obj<</Title( endobj\n'.repeat(n),
    after: `5 0 obj<</Title(endobj x)/Subject(endobj ${' '.repeat(20000)})/Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding<</Differences[116/x]>>>>endobj\n`
  });

const cmapPdf = (body) => makePdf([['Hi']], { type0: true, cmap: cmapOf(body) });

// A saved state holds the font. After `saves` saves in F1, F2 is set and saved once more, then F1 again, and the
// restore must bring back F2: only F2 maps <0001>, to H.
const savedFont = (saves) =>
  onePage(`${START}${'q '.repeat(saves)}BT /F2 12 Tf ET q BT /F1 12 Tf ET Q BT 72 700 Td <0001> Tj ET`, {
    fonts: '/F1 5 0 R /F2 11 0 R',
    objects: `11 0 obj<<${TYPE0}/ToUnicode 12 0 R>>endobj\n${streamObject(12, '', cmapOf('1 beginbfchar <0001> <0048> endbfchar'))}`
  });

// Limits of the reader that drop content, each in a small PDF, with what it reads before the limit: each must come
// back partial. And the same limits not reached, which must not.
function silentCaps() {
  return {
    capped: {
      formDepth: { pdf: nestedForms(5), read: 'start' },
      tjItems: { pdf: onePage(`BT /F1 12 Tf 72 720 Td [(start) ${'1 '.repeat(100000)}(lost)] TJ ET`), read: 'start' },
      dictItems: { pdf: onePage(START, { resources: `/Extra<<${Array.from({ length: 100001 }, (_, i) => `/K${i} 1`).join('')}>>` }), read: 'start' },
      nesting: { pdf: onePage(`${START}${'['.repeat(70)}(lost)${']'.repeat(70)} pop`), read: 'start' },
      operands: { pdf: onePage(`${START}BT /F1 12 Tf 72 700 Td ${'1 '.repeat(70)}(lost) Tj ET`), read: 'start' },
      noFont: { pdf: onePage(`${START}BT /F9 12 Tf 72 700 Td (lost) Tj ET`), read: 'start' },
      inlineImage: { pdf: onePage(`${START}BI /W 1 /H 1 /BPC 8 /CS /G ID xyz ${line('lost')}`), read: 'start' },
      lzw: { pdf: onePage(START, { contents: '[4 0 R 10 0 R]', objects: streamObject(10, '/Filter/LZWDecode', 'garbage') }), read: 'start' },
      corruptFlate: { pdf: onePage(START, { contents: '[4 0 R 10 0 R]', objects: streamObject(10, '/Filter/FlateDecode', 'not deflate data at all') }), read: 'start' },
      missingContents: { pdf: onePage(START, { contents: '[4 0 R 99 0 R]' }), read: 'start' },
      refChain: { pdf: refChain(10), read: 'start' },
      deepTree: { pdf: deepTree(70), read: 'start' },
      cmapLongCode: { pdf: cmapPdf('1 beginbfchar <0000000001> <0041> endbfchar 1 beginbfchar <0001> <0048> endbfchar'), read: 'H' },
      cmapWideRange: { pdf: cmapPdf('1 beginbfrange <00000000> <0001FFFF> <0041> endbfrange 1 beginbfchar <0001> <0048> endbfchar'), read: 'H' },
      cmapRanges: { pdf: cmapPdf(`${Array.from({ length: 33 }, (_, i) => `1 begincodespacerange <${(i + 2).toString(16).padStart(4, '0')}00> <${(i + 2).toString(16).padStart(4, '0')}FF> endcodespacerange`).join('\n')}\n1 beginbfchar <0001> <0048> endbfchar`), read: 'H' },
      cmapLongText: { pdf: cmapPdf(`1 beginbfchar <0001> <${'0048'.repeat(300)}> endbfchar`), read: 'HHHH' },
      savedStates: { pdf: savedFont(64), read: 'start' },
      // Strings that never close, each looking through the rest of the file for its end: once they have looked
      // through it four times, the strings after them stay cut.
      openStrings: { pdf: openStrings(5), read: 'start' },
      // A string that ran on to its end doesn't undo a later one in the same object that room left cut.
      spentStrings: { pdf: spentStrings(4), read: 'start' }
    },
    // Right at the limits: read in full.
    within: {
      formDepth: { pdf: nestedForms(4), read: 'deepest' },
      tjItems: { pdf: onePage(`BT /F1 12 Tf 72 720 Td [(start) ${'1 '.repeat(99998)}(last)] TJ ET`), read: 'last' },
      refChain: { pdf: refChain(6), read: 'lost' },
      deepTree: { pdf: deepTree(60), read: 'deep' },
      savedStates: { pdf: savedFont(63), read: 'H' },
      openStrings: { pdf: openStrings(4), read: 'start' },
      spentStrings: { pdf: spentStrings(3), read: 'sxarx' }
    }
  };
}

module.exports = { makePdf, hostilePdfs, manyOpsPdf, silentCaps, onePage, plainPage };
