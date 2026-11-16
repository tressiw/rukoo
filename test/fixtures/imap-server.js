'use strict';

// An IMAP server for tests, over plain TCP on this PC. It greets, answers CAPABILITY and answers LOGIN with the
// response it was given, such as Gmail's refusal of a normal Google password. Given a mailbox, it also lists, opens and
// fetches from that one folder, enough for a sync. Any other command gets OK. It only reads single-line commands, which
// is all a LOGIN with a plain username and password takes.
const net = require('net');

const quote = (s) => `"${String(s).replace(/(["\\])/g, '\\$1')}"`;
const address = ([name, email]) => {
  const [mailbox, host] = email.split('@');
  return `((${quote(name)} NIL ${quote(mailbox)} ${quote(host)}))`;
};

// A single-part text/plain BODYSTRUCTURE of size bytes.
const textStructure = (size) => `("text" "plain" ("charset" "utf-8") NIL NIL "7bit" ${size} 1 NIL NIL NIL NIL)`;

// A BODYSTRUCTURE with a text part inside depth multiparts, each holding the next: the shape of a crafted message.
function nestedStructure(depth) {
  let structure = textStructure(5);
  for (let i = 0; i < depth; i++) structure = `(${structure} "mixed")`;
  return structure;
}

// The uids or sequence numbers a sequence set such as 1:*, 7,9 or 3:5 names, out of all.
function inSet(set, all) {
  const max = all.length ? Math.max(...all) : 0;
  const picked = new Set();
  for (const part of set.split(',')) {
    const [a, b = a] = part.split(':').map((n) => (n === '*' ? max : Number(n)));
    for (const n of all) if (n >= Math.min(a, b) && n <= Math.max(a, b)) picked.add(n);
  }
  return picked;
}

// One message's answer to a FETCH, with the items asked in the order asked.
function fetchItems(m, items) {
  const out = [];
  for (const item of items) {
    const upper = item.toUpperCase();
    if (upper === 'UID') out.push(`UID ${m.uid}`);
    else if (upper === 'FLAGS') out.push(`FLAGS (${(m.flags || []).join(' ')})`);
    else if (upper === 'INTERNALDATE') out.push(`INTERNALDATE "07-Nov-2026 10:00:00 +0000"`);
    else if (upper === 'RFC822.SIZE') out.push(`RFC822.SIZE ${m.size || 100}`);
    else if (upper === 'BODYSTRUCTURE') out.push(`BODYSTRUCTURE ${m.bodyStructure}`);
    else if (upper === 'ENVELOPE') {
      const from = address(m.from || ['Sender', 'sender@example.com']);
      out.push(`ENVELOPE ("Sat, 07 Nov 2026 10:00:00 +0000" ${quote(m.subject || '')} ${from} ${from} ${from} ${address(['Me', 'me@example.com'])} NIL NIL NIL ${quote(`<${m.uid}@example.com>`)})`);
    } else {
      // BODY.PEEK[part]<0.max>: the start of a part, as a literal.
      const body = /^BODY\.PEEK\[([^\]]*)\](?:<(\d+)\.(\d+)>)?$/.exec(upper);
      if (!body) continue;
      const text = String((m.parts || {})[body[1]] || '').slice(0, body[3] ? Number(body[3]) : undefined);
      out.push(`BODY[${body[1]}]${body[2] ? `<${body[2]}>` : ''} {${Buffer.byteLength(text)}}\r\n${text}`);
    }
  }
  return out.join(' ');
}

// login: the status and text after the tag in the answer to LOGIN.
// mailbox: { path, uidValidity, messages: [{ uid, flags, subject, from: [name, email], size, bodyStructure, parts }] },
// the one folder there is. bodyStructure is the IMAP text of the message's BODYSTRUCTURE; parts maps a part number to
// its text. Every command but LOGIN goes in commands, without its tag.
function imapServer({ login = 'OK LOGIN completed', mailbox = null } = {}) {
  const sockets = new Set();
  const logins = [];
  const commands = [];
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.on('error', () => {});
    socket.setEncoding('utf8');
    socket.write('* OK [CAPABILITY IMAP4rev1] Test server ready\r\n');
    let buffer = '';
    const fetch = (tag, byUid, set, list) => {
      const items = list.replace(/^\(|\)$/g, '').match(/BODY\.PEEK\[[^\]]*\](?:<[\d.]+>)?|[^\s()]+/gi) || [];
      const messages = mailbox.messages;
      const picked = inSet(set, byUid ? messages.map((m) => m.uid) : messages.map((_, i) => i + 1));
      messages.forEach((m, i) => {
        if (picked.has(byUid ? m.uid : i + 1)) socket.write(`* ${i + 1} FETCH (${fetchItems(m, byUid ? ['UID', ...items.filter((x) => x.toUpperCase() !== 'UID')] : items)})\r\n`);
      });
      socket.write(`${tag} OK ${byUid ? 'UID FETCH' : 'FETCH'} completed\r\n`);
    };
    socket.on('data', (chunk) => {
      buffer += chunk;
      let end;
      while ((end = buffer.indexOf('\r\n')) >= 0) {
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const [tag, command = ''] = line.split(' ');
        const rest = line.slice(tag.length + command.length + 2);
        if (command.toUpperCase() !== 'LOGIN') commands.push(line.slice(tag.length + 1));
        switch (command.toUpperCase()) {
          case 'CAPABILITY':
            socket.write(`* CAPABILITY IMAP4rev1\r\n${tag} OK CAPABILITY completed\r\n`);
            break;
          case 'LOGIN':
            logins.push(line);
            socket.write(`${tag} ${login}\r\n`);
            break;
          case 'LOGOUT':
            socket.end(`* BYE Logging out\r\n${tag} OK LOGOUT completed\r\n`);
            break;
          case 'LIST':
            if (mailbox) socket.write(`* LIST (\\HasNoChildren) "/" ${quote(mailbox.path || 'INBOX')}\r\n`);
            socket.write(`${tag} OK LIST completed\r\n`);
            break;
          case 'SELECT':
          case 'EXAMINE':
            if (!mailbox) {
              socket.write(`${tag} OK\r\n`);
              break;
            }
            socket.write(
              `* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* ${mailbox.messages.length} EXISTS\r\n* OK [UIDVALIDITY ${mailbox.uidValidity || 1}] UIDs valid\r\n` +
                `* OK [UIDNEXT ${Math.max(0, ...mailbox.messages.map((m) => m.uid)) + 1}] Predicted next UID\r\n` +
                `${tag} OK [${command.toUpperCase() === 'EXAMINE' ? 'READ-ONLY' : 'READ-WRITE'}] ${command.toUpperCase()} completed\r\n`
            );
            break;
          case 'FETCH': {
            const [set, ...list] = rest.split(' ');
            if (mailbox) fetch(tag, false, set, list.join(' '));
            else socket.write(`${tag} OK\r\n`);
            break;
          }
          case 'UID': {
            const [sub, set, ...list] = rest.split(' ');
            if (mailbox && sub.toUpperCase() === 'FETCH') fetch(tag, true, set, list.join(' '));
            else socket.write(`${tag} OK\r\n`);
            break;
          }
          default:
            socket.write(`${tag} OK\r\n`);
        }
      }
    });
  });
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () =>
      resolve({
        port: server.address().port,
        logins,
        commands,
        close: () =>
          new Promise((done) => {
            for (const s of sockets) s.destroy();
            server.close(() => done());
          })
      })
    );
  });
}

// What Gmail answers to LOGIN with a normal Google password.
const GMAIL_APP_PASSWORD_REQUIRED = 'NO [ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)';

module.exports = { imapServer, nestedStructure, textStructure, GMAIL_APP_PASSWORD_REQUIRED };
