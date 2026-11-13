'use strict';

// HTML signatures: cleaning, how they are stored next to plain-text ones, and what a sent mail carries.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const { simpleParser } = require('mailparser');
const { tempDir } = require('./tmp');
const { Engine } = require('../src/main/engine');
const { cleanSignatureHtml, signatureText } = require('../src/main/signature');

// A 1x1 PNG.
const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';

const SIGNATURE = `<table cellpadding="0" style="border-collapse:collapse"><tr>
  <td valign="top" style="padding-right:12px"><img src="${PNG}" alt="Rukoo" width="48"></td>
  <td><b style="color:#1c2340">Bart van der Meeren</b><br><span style="color:#888;font-size:12px">Founder</span><br>
  <a href="https://github.com/MajesteitBart/rukoo">rukoo on GitHub</a> | <a href="tel:+31201234567">+31 20 123 4567</a></td>
</tr></table>`;

test('a signature keeps its formatting, links, tables and images', () => {
  const clean = cleanSignatureHtml(SIGNATURE);
  assert.match(clean, /<table cellpadding="0" style="border-collapse:collapse">/);
  assert.match(clean, /<td valign="top" style="padding-right:12px">/);
  assert.match(clean, /<b style="color:#1c2340">Bart van der Meeren<\/b>/);
  assert.match(clean, /<span style="color:#888;font-size:12px">Founder<\/span>/);
  assert.match(clean, /<a href="https:\/\/github.com\/MajesteitBart\/rukoo">rukoo on GitHub<\/a>/);
  assert.match(clean, /<a href="tel:\+31201234567">/);
  assert.ok(clean.includes(`<img src="${PNG}" alt="Rukoo" width="48">`));
  assert.ok(cleanSignatureHtml('<img src="https://example.com/logo.png">').includes('src="https://example.com/logo.png"'));
  assert.equal(cleanSignatureHtml('<font color="#336699" face="Arial">x</font>'), '<font color="#336699" face="Arial">x</font>');
  // Cleaning what was cleaned changes nothing, so a stored signature survives every load.
  assert.equal(cleanSignatureHtml(clean), clean);
});

test('unsafe HTML is stripped from a signature', () => {
  const dirty = `<html><head><style>p{color:red}</style><meta http-equiv="refresh" content="0;url=https://x"></head><body>
    <!--[if mso]><p>conditional</p><![endif]-->
    <p class="MsoNormal signature" id="x" data-c="send" onclick="alert(1)" style="color:red;position:fixed;background:url(https://track.example/p.gif);behavior:url(x.htc);width:expression(alert(1))">Hi<o:p></o:p></p>
    <a href="javascript:alert(1)">js link</a><a href=" JaVaScRiPt:alert(1)">js2</a><a href="data:text/html,<script>alert(1)</script>">data link</a>
    <img src="x" onerror="alert(1)"><img src="file:///C:/Users/me/logo.png"><img src="cid:image001.png@01D">
    <img src="data:image/svg+xml;base64,PHN2Zz48L3N2Zz4="><img src="data:text/html;base64,PHNjcmlwdD4=">
    <script>alert(1)</script><iframe src="https://example.com"></iframe><form action="https://x"><input name="a"><button>Go</button></form>
    <svg><script>alert(1)</script></svg><object data="x"></object><embed src="x"><template><b>t</b></template>
    <textarea>&lt;/textarea&gt;<img src=x onerror=alert(1)></textarea><noscript><b>n</b></noscript>
    &lt;script&gt;alert(1)&lt;/script&gt;
  </body></html>`;
  const clean = cleanSignatureHtml(dirty);
  for (const bad of ['<script', '<style', '<meta', '<iframe', '<form', '<input', '<button', '<svg', '<object', '<embed', '<template', '<textarea', '<noscript', 'onclick', 'onerror', 'javascript:', 'class=', 'id=', 'data-c', 'url(', 'expression', 'behavior', 'position', 'file:', 'cid:', 'svg+xml', 'data:text', 'conditional', 'o:p']) {
    assert.ok(!clean.toLowerCase().includes(bad.toLowerCase()), `${bad} in ${clean}`);
  }
  assert.ok(clean.includes('<p style="color:red">Hi</p>'), clean);
  // Links that aren't allowed keep their text; escaped markup stays text.
  assert.ok(clean.includes('js link') && clean.includes('data link'));
  assert.ok(clean.includes('&lt;script&gt;alert(1)&lt;/script&gt;'));
  assert.ok(!/<img/.test(clean));
});

test('attribute values and text cannot break out of the markup', () => {
  const clean = cleanSignatureHtml('<a href="https://x.nl/?a=&quot;&gt;&lt;script&gt;" title="&quot; onmouseover=&quot;x">t</a><img src="https://x.nl/a.png" alt="&quot;><script>">');
  assert.equal((clean.match(/<a /g) || []).length, 1);
  assert.ok(!/<script/i.test(clean));
  assert.ok(clean.includes('href="https://x.nl/?a=&quot;&gt;&lt;script&gt;"'));
  assert.ok(clean.includes('title="&quot; onmouseover=&quot;x"'));
});

test('an empty signature is no signature; one that is too large is refused', () => {
  assert.equal(cleanSignatureHtml(''), '');
  assert.equal(cleanSignatureHtml('<div><br></div> <p>&nbsp;</p>'), '');
  assert.equal(cleanSignatureHtml(null), '');
  assert.equal(cleanSignatureHtml(`<img src="${PNG}">`), `<img src="${PNG}">`);
  const huge = `<img src="data:image/png;base64,${'A'.repeat(1100 * 1024)}">`;
  assert.throws(() => cleanSignatureHtml(huge), /too large/);
});

test('the text of a signature reads well', () => {
  assert.equal(signatureText(cleanSignatureHtml(SIGNATURE)), 'Bart van der Meeren\nFounder\nrukoo on GitHub | +31 20 123 4567');
  assert.equal(signatureText('Groet,<br>Bart'), 'Groet,\nBart');
});

test('a plain-text signature from an earlier version stays plain text', () => {
  const dir = tempDir('sem-sig-');
  fs.writeFileSync(path.join(dir, 'settings.json'), JSON.stringify({ signature: 'Groet,\nBart', signatureOptIn: true }));
  const engine = new Engine({ dataDir: dir }).init();
  assert.equal(engine.settings.signature, 'Groet,\nBart');
  assert.equal(engine.settings.signatureHtml, '');
});

test('a formatted signature is stored cleaned, with its text for earlier versions', () => {
  const dir = tempDir('sem-sig-');
  const engine = new Engine({ dataDir: dir }).init();
  engine.updateSettings({ signatureHtml: '<b onclick="x()">Bart</b><br><a href="https://rukoo.app">rukoo.app</a><script>x</script>' });
  assert.equal(engine.settings.signatureHtml, '<b>Bart</b><br><a href="https://rukoo.app">rukoo.app</a>');
  assert.equal(engine.settings.signature, 'Bart\nrukoo.app');
  const stored = JSON.parse(fs.readFileSync(path.join(dir, 'settings.json'), 'utf8'));
  assert.equal(stored.signatureHtml, '<b>Bart</b><br><a href="https://rukoo.app">rukoo.app</a>');
  // Plain text again drops the HTML.
  engine.updateSettings({ signature: 'Just text' });
  assert.equal(engine.settings.signatureHtml, '');
  assert.equal(engine.settings.signature, 'Just text');
  assert.throws(() => engine.updateSettings({ signatureHtml: `<img src="data:image/png;base64,${'A'.repeat(1100 * 1024)}">` }), /too large/);
  assert.equal(engine.settings.signature, 'Just text');
  // A hand-edited settings file is cleaned when it loads.
  fs.writeFileSync(path.join(dir, 'settings.json'), JSON.stringify({ ...stored, signatureHtml: '<i>x</i><iframe src="https://x"></iframe>' }));
  assert.equal(new Engine({ dataDir: dir }).init().settings.signatureHtml, '<i>x</i>');
});

test('an account signature can be formatted, and reset to the general one', async () => {
  const dir = tempDir('sem-sig-');
  const engine = new Engine({ dataDir: dir }).init();
  const acc = await engine.addAccount({ type: 'demo' });
  assert.equal(acc.signature, null);
  assert.equal(acc.signatureHtml, null);
  let pub = await engine.updateAccount(acc.id, { signatureHtml: '<p style="color:#123456">Demo <u>team</u></p><img src="x" onerror="y">' });
  pub = engine.state().accounts[0];
  assert.equal(pub.signatureHtml, '<p style="color:#123456">Demo <u>team</u></p>');
  assert.equal(pub.signature, 'Demo team');
  // A signature that is too large leaves the account as it was.
  await assert.rejects(engine.updateAccount(acc.id, { name: 'Other', signatureHtml: `<img src="data:image/png;base64,${'A'.repeat(1100 * 1024)}">` }), /too large/);
  assert.equal(engine.state().accounts[0].name, acc.name);
  assert.equal(engine.state().accounts[0].signature, 'Demo team');
  // The earlier way of saving plain text still works, and drops the HTML.
  await engine.updateAccount(acc.id, { signature: 'Plain' });
  assert.equal(engine.state().accounts[0].signatureHtml, null);
  assert.equal(engine.state().accounts[0].signature, 'Plain');
  await engine.updateAccount(acc.id, { signature: null });
  assert.equal(engine.state().accounts[0].signature, null);
  assert.equal(engine.state().accounts[0].signatureHtml, null);
  await engine.close();
});

test('a sent mail and a saved draft carry the signature image inline and its text in the plain part', async () => {
  const dir = tempDir('sem-sig-');
  const engine = new Engine({ dataDir: dir }).init();
  const acc = await engine.addAccount({ type: 'demo' });
  await engine.syncAccount(acc.id);
  const html = `<div style="font-family:'Segoe UI',Arial,sans-serif;font-size:12pt"><div>Hello</div><div><br></div><div class="signature">${cleanSignatureHtml(SIGNATURE)}</div></div>`;
  const raw = await engine.session(engine.account(acc.id)).send(await engine.buildMail(engine.account(acc.id), { to: ['demo@example.com'], subject: 'Signed', html }));
  const parsed = await simpleParser(raw, { keepCidLinks: true });
  const image = parsed.attachments.find((a) => a.contentType === 'image/png');
  assert.ok(image, 'the data image is an inline attachment');
  assert.equal(image.contentDisposition, 'inline');
  assert.ok(parsed.html.includes(`cid:${image.cid}`), 'the html points at it');
  assert.ok(!parsed.html.includes('data:image'));
  assert.match(parsed.html, /<a href="https:\/\/github.com\/MajesteitBart\/rukoo">rukoo on GitHub<\/a>/);
  assert.match(parsed.text, /Hello\n+Bart van der Meeren\nFounder\nrukoo on GitHub \| \+31 20 123 4567/);

  // A draft keeps it too, and opens with the image back in place.
  const draftId = await engine.saveDraft({ accountId: acc.id, to: ['a@example.com'], subject: 'Signed draft', html });
  const draft = await engine.getMessage(draftId);
  assert.ok(draft.html.includes('class="signature"'));
  assert.ok(draft.html.includes('data:image/png;base64,'), 'the inline image is back as data');
  assert.ok(draft.html.includes('rukoo on GitHub'));
  assert.equal(draft.attachments.length, 0, 'the image is not listed as an attachment');
  await engine.close();
});

test('inline images become Content-ID parts, once per picture', () => {
  const util = require('../src/main/mailutil');
  const { html, attachments } = util.inlineDataImages(
    `<p><img src="${PNG}" alt="a"> <img alt="b" src='${PNG}'> <img src="data:image/jpeg;base64,/9j/4AAQ"> <img src="https://example.com/x.png"></p>`
  );
  assert.equal(attachments.length, 2);
  assert.deepEqual(
    attachments.map((a) => [a.contentType, a.filename, a.contentDisposition]),
    [
      ['image/png', 'image-1.png', 'inline'],
      ['image/jpeg', 'image-2.jpg', 'inline']
    ]
  );
  assert.equal(html.match(new RegExp(`cid:${attachments[0].cid}`, 'g')).length, 2);
  assert.ok(html.includes(`src="cid:${attachments[1].cid}"`));
  assert.ok(html.includes('src="https://example.com/x.png"'));
  assert.ok(!html.includes('data:'));
  assert.equal(attachments[0].content.subarray(1, 4).toString(), 'PNG');
});

test('an inline image keeps the size it was given in the editor', () => {
  const util = require('../src/main/mailutil');
  const { html, attachments } = util.inlineDataImages(`<div><img width="200" src="${PNG}" style="height: auto; max-width: 100%; width: 200px;"></div>`);
  assert.equal(html, `<div><img width="200" src="cid:${attachments[0].cid}" style="height: auto; max-width: 100%; width: 200px;"></div>`);
});
