'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { simpleParser } = require('mailparser');
const MailComposer = require('nodemailer/lib/mail-composer');
const util = require('../src/main/mailutil');
const { serverView } = require('./fixtures/mime-server');

test('folderRole uses special-use flags and Dutch/English names', () => {
  assert.equal(util.folderRole({ path: 'INBOX' }), 'inbox');
  assert.equal(util.folderRole({ path: '[Gmail]/Verzonden berichten', specialUse: '\\Sent' }), 'sent');
  assert.equal(util.folderRole({ path: 'Verwijderde items', name: 'Verwijderde items' }), 'trash');
  assert.equal(util.folderRole({ path: 'INBOX.Drafts', name: 'Drafts' }), 'drafts');
  assert.equal(util.folderRole({ path: 'Klanten', name: 'Klanten' }), null);
});

test('findPreviewPart prefers text/plain and skips attachments', () => {
  const structure = {
    type: 'multipart/mixed',
    childNodes: [
      {
        part: '1',
        type: 'multipart/alternative',
        childNodes: [
          { part: '1.1', type: 'text/plain', encoding: 'quoted-printable', parameters: { charset: 'utf-8' } },
          { part: '1.2', type: 'text/html', encoding: 'base64' }
        ]
      },
      { part: '2', type: 'text/plain', disposition: 'attachment', dispositionParameters: { filename: 'notes.txt' } }
    ]
  };
  assert.equal(util.findPreviewPart(structure).part, '1.1');
  assert.equal(util.hasAttachments(structure), true);
  assert.equal(util.partKey({ type: 'text/plain' }), '1');
});

test('previewFromPart decodes quoted-printable, base64 and html', () => {
  const qp = Buffer.from('Caf=C3=A9 at 10:00 =\r\ntomorrow');
  assert.equal(util.previewFromPart(qp, { type: 'text/plain', encoding: 'quoted-printable', parameters: { charset: 'utf-8' } }), 'Café at 10:00 tomorrow');
  // Truncated base64 (partial fetch) must not throw.
  const b64 = Buffer.from(Buffer.from('<p>Hello <b>there</b></p><style>x{}</style>').toString('base64').slice(0, 30));
  const out = util.previewFromPart(b64, { type: 'text/html', encoding: 'base64' });
  assert.match(out, /^Hello/);
  const latin = Buffer.from([0x63, 0x61, 0x66, 0xe9]);
  assert.equal(util.previewFromPart(latin, { type: 'text/plain', encoding: '8bit', parameters: { charset: 'iso-8859-1' } }), 'café');
});

test('sanitizeHtml strips scripts, handlers and javascript urls', () => {
  const dirty = '<div onclick="x()">a<script>alert(1)</script><a href="javascript:alert(1)">l</a><iframe src="x"></iframe><img src=x onerror=alert(1)></div>';
  const clean = util.sanitizeHtml(dirty);
  assert.ok(!/script|onclick|onerror|javascript:|iframe/i.test(clean), clean);
  assert.ok(clean.includes('<a href="#">l</a>'));
});

test('textToHtml escapes and links urls', () => {
  const html = util.textToHtml('<b> see https://example.com/a?b=1.');
  assert.ok(html.includes('&lt;b&gt;'));
  assert.ok(html.includes('<a href="https://example.com/a?b=1">'));
});

// ---------- a large email's text before its attachments (#56) ----------

const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64');
const MB = 1024 * 1024;
const compose = (mail) => new MailComposer({ from: 'Sanne <sanne@example.com>', to: 'demo@example.com', subject: 'Photos', ...mail }).compile().build();

// What the reader gets of an email before its attachments: the skeleton from the parts readableParts names, the way a
// server answers them. null when the email doesn't split.
function skeletonOf(raw) {
  const view = serverView(raw);
  const keys = util.readableParts(view.structure, view.size);
  if (!keys) return null;
  const parts = new Map();
  for (const key of keys) {
    const found = view.sections.get(key.toLowerCase());
    if (found) parts.set(key.toLowerCase(), found);
  }
  return { keys, source: util.skeletonSource(view.header, view.structure, parts) };
}

// The fields of mailparser's attachments that don't depend on their content.
const shape = (list) => list.map((a) => ({ filename: a.filename, contentType: a.contentType, partId: a.partId, cid: a.cid, related: Boolean(a.related), disposition: a.contentDisposition }));
const omittedSize = (a) => a.headers.get(util.OMITTED.toLowerCase());

// The skeleton reads like the whole email: the same text, the same attachments in the same order, and the ones left
// empty name about their decoded size.
async function sameAsWhole(raw) {
  const split = skeletonOf(raw);
  assert.ok(split, 'the email splits');
  const whole = await simpleParser(raw);
  const part = await simpleParser(split.source);
  assert.equal(part.text, whole.text);
  assert.equal(part.subject, whole.subject);
  assert.deepEqual(shape(part.attachments), shape(whole.attachments));
  part.attachments.forEach((a, i) => {
    // An empty part in an 8bit encoding reads as a line break, so what was left out is known by the header.
    if (omittedSize(a) === undefined) return assert.ok(a.content.equals(whole.attachments[i].content), `${a.filename} kept whole`);
    const size = Number(omittedSize(a));
    // Estimated from the encoded size, for base64 in lines of 76 characters; other line lengths are a few percent off.
    assert.ok(Math.abs(size - whole.attachments[i].size) <= Math.max(8, whole.attachments[i].size * 0.03), `${a.filename}: ${size} for ${whole.attachments[i].size}`);
  });
  return { split, whole, part };
}

test('a large email splits into its text, its inline pictures and empty attachments that mailparser lists like the whole email', async () => {
  const raw = await compose({
    text: 'See the photos.',
    html: '<p>See the photos.</p><img src="cid:pic@example.com">',
    attachments: [
      { filename: 'pic.png', content: PNG, cid: 'pic@example.com' },
      { filename: 'report.pdf', content: Buffer.alloc(3 * MB, 7), contentType: 'application/pdf' },
      { filename: 'photo.png', content: PNG },
      { filename: 'notes.txt', content: 'attached text', contentDisposition: 'attachment' }
    ]
  });
  const { split, whole, part } = await sameAsWhole(raw);
  // The inline picture came along, so the html is the whole email's, picture included.
  assert.equal(part.html, whole.html);
  assert.match(part.html, /data:image\/png;base64,iVBOR/);
  assert.equal(part.attachments.find((a) => a.filename === 'report.pdf').size, 0);
  assert.ok(omittedSize(part.attachments.find((a) => a.filename === 'notes.txt')), 'a text attachment stays out too');
  assert.ok(split.source.length < 64 * 1024, `the skeleton is small: ${split.source.length} bytes`);
  // Every part's header, and the bodies of the text and the inline picture only.
  const view = serverView(raw);
  const bodies = split.keys.filter((k) => !/\.MIME$/.test(k)).map((k) => view.sections.get(k).length);
  assert.ok(bodies.every((n) => n < 1024), `${bodies}`);
});

test('inline pictures beyond the budget stay out, and their place in the html stays empty', async () => {
  const big = Buffer.concat([PNG, Buffer.alloc(util.INLINE_IMAGES, 1)]);
  const raw = await compose({
    html: '<p>Banner:</p><img src="cid:banner@example.com"><img src="cid:logo@example.com">',
    attachments: [
      { filename: 'banner.png', content: big, cid: 'banner@example.com' },
      { filename: 'logo.png', content: PNG, cid: 'logo@example.com' },
      { filename: 'report.pdf', content: Buffer.alloc(3 * MB, 7), contentType: 'application/pdf' }
    ]
  });
  const { part } = await sameAsWhole(raw);
  assert.match(part.html, /src="data:image\/png;base64,"/, 'the banner is not in');
  assert.match(part.html, /src="data:image\/png;base64,iVBOR/, 'the logo is');
});

test('an invitation keeps its calendar parts, and a forwarded email its place among the attachments', async () => {
  const ics = 'BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:1@example.com\r\nSUMMARY:Call\r\nDTSTART:20270101T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n';
  const forwarded = await compose({ subject: 'Earlier', text: 'The earlier email' });
  const raw = await compose({
    text: 'Join the call.',
    html: '<p>Join the call.</p>',
    icalEvent: { method: 'REQUEST', content: ics },
    attachments: [
      // Nodemailer would mark it inline, which mailparser shows as text (the next test).
      { filename: 'earlier.eml', content: forwarded, contentType: 'message/rfc822', contentDisposition: 'attachment' },
      { filename: 'deck.pdf', content: Buffer.alloc(2 * MB, 3), contentType: 'application/pdf' }
    ]
  });
  const { part } = await sameAsWhole(raw);
  const calendar = part.attachments.filter((a) => /calendar|ics/.test(a.contentType));
  assert.ok(calendar.length >= 1);
  for (const a of calendar) assert.match(a.content.toString(), /SUMMARY:Call/);
  assert.ok(omittedSize(part.attachments.find((a) => a.contentType === 'message/rfc822')), 'a forwarded email is an attachment and stays out');
});

test('an email forwarded inline reads as text, so it comes along', async () => {
  const inner = (await compose({ subject: 'Earlier', text: 'The earlier email' })).toString();
  const raw = [
    'From: sanne@example.com',
    'To: demo@example.com',
    'Subject: Fwd: Earlier',
    'MIME-Version: 1.0',
    'Content-Type: multipart/mixed; boundary="outer"',
    '',
    '--outer',
    'Content-Type: text/plain; charset=utf-8',
    '',
    'See below.',
    '--outer',
    'Content-Type: message/rfc822',
    'Content-Disposition: inline',
    '',
    inner,
    '--outer',
    'Content-Type: application/zip',
    'Content-Transfer-Encoding: base64',
    'Content-Disposition: attachment; filename="photos.zip"',
    '',
    Buffer.alloc(2 * MB, 9).toString('base64').replace(/.{76}/g, '$&\r\n'),
    '--outer--',
    ''
  ].join('\r\n');
  const { part } = await sameAsWhole(raw);
  assert.match(part.text, /The earlier email/);
});

test('a signed email keeps its structure and its signature part stays out', async () => {
  const raw = [
    'From: sanne@example.com',
    'To: demo@example.com',
    'Subject: Signed',
    'MIME-Version: 1.0',
    'Content-Type: multipart/signed; protocol="application/pkcs7-signature"; micalg=sha-256; boundary="sig"',
    '',
    '--sig',
    'Content-Type: multipart/mixed; boundary="body"',
    '',
    '--body',
    'Content-Type: text/plain; charset=utf-8',
    'Content-Transfer-Encoding: quoted-printable',
    '',
    'Caf=C3=A9 tomorrow.',
    '--body',
    'Content-Type: application/pdf; name="plan.pdf"',
    'Content-Transfer-Encoding: base64',
    'Content-Disposition: attachment; filename="plan.pdf"',
    '',
    Buffer.alloc(2 * MB, 4).toString('base64').replace(/.{76}/g, '$&\r\n'),
    '--body--',
    '',
    '--sig',
    'Content-Type: application/pkcs7-signature; name="smime.p7s"',
    'Content-Transfer-Encoding: base64',
    'Content-Disposition: attachment; filename="smime.p7s"',
    '',
    Buffer.alloc(2000, 6).toString('base64'),
    '--sig--',
    ''
  ].join('\r\n');
  const { part } = await sameAsWhole(raw);
  assert.equal(part.text, 'Café tomorrow.');
  assert.deepEqual(part.attachments.map((a) => a.filename), ['plan.pdf', 'smime.p7s']);
});

test('an email does not split when it is one part or mostly text, and not without a boundary or a part header', async () => {
  assert.equal(skeletonOf(await compose({ html: `<p>${'x'.repeat(2 * MB)}</p>` })), null);
  assert.equal(skeletonOf(await compose({ text: 'y'.repeat(2 * MB), attachments: [{ filename: 'a.pdf', content: Buffer.alloc(MB, 1) }] })), null);
  const view = serverView(await compose({ text: 'Hi', attachments: [{ filename: 'a.pdf', content: Buffer.alloc(2 * MB, 1) }] }));
  assert.ok(util.readableParts(view.structure, view.size));
  const boundary = view.structure.parameters.boundary;
  delete view.structure.parameters.boundary;
  assert.throws(() => util.skeletonSource(view.header, view.structure, view.sections), /boundary/);
  view.structure.parameters.boundary = boundary;
  view.sections.delete('1.mime');
  assert.throws(() => util.skeletonSource(view.header, view.structure, view.sections), /header for part 1/);
});

test('decodedSize undoes base64 and leaves other encodings alone', () => {
  const content = Buffer.alloc(100000, 5);
  const encoded = content.toString('base64').replace(/.{76}/g, '$&\r\n');
  assert.ok(Math.abs(util.decodedSize(encoded.length, 'base64') - content.length) < 60);
  assert.equal(util.decodedSize(1234, 'quoted-printable'), 1234);
  assert.equal(util.decodedSize(undefined, '7bit'), 0);
});
