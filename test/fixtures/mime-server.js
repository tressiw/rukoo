'use strict';

// What an IMAP server makes of a message, for tests of fetching one in parts: its BODYSTRUCTURE in the shape
// imapflow parses it into, and the sections a FETCH asks for (HEADER, n.MIME and n). It reads as much MIME as the
// fixtures use: boundaries, folded headers and quoted parameters, without RFC 2231 continuations.

const CRLF = Buffer.from('\r\n');

// The header and body of a part, the header with the empty line that ends it.
function split(buf) {
  const end = buf.indexOf('\r\n\r\n');
  return end < 0 ? [buf, Buffer.alloc(0)] : [buf.subarray(0, end + 4), buf.subarray(end + 4)];
}

function fields(header) {
  const out = {};
  for (const line of header.toString('latin1').replace(/\r\n[ \t]+/g, ' ').split('\r\n')) {
    const i = line.indexOf(':');
    if (i > 0) out[line.slice(0, i).trim().toLowerCase()] = line.slice(i + 1).trim();
  }
  return out;
}

// "text/plain; charset=utf-8; name=\"a b.txt\"" as its value and its parameters.
function params(value) {
  const [main, ...rest] = String(value || '').split(';');
  const out = {};
  for (const p of rest) {
    const i = p.indexOf('=');
    if (i > 0) out[p.slice(0, i).trim().toLowerCase()] = p.slice(i + 1).trim().replace(/^"(.*)"$/, '$1');
  }
  return { value: main.trim().toLowerCase(), params: out };
}

// The parts of a multipart body, without the CRLF before each boundary, which belongs to the boundary.
function children(body, boundary) {
  const out = [];
  const open = Buffer.from(`--${boundary}`);
  let pos = body.indexOf(open);
  while (pos >= 0) {
    const after = body.subarray(pos + open.length, pos + open.length + 2).toString();
    if (after === '--') break;
    const start = body.indexOf(CRLF, pos) + 2;
    const next = body.indexOf(Buffer.from(`\r\n--${boundary}`), start);
    out.push(body.subarray(start, next < 0 ? body.length : next));
    pos = next < 0 ? -1 : next + 2;
  }
  return out;
}

function serverView(raw) {
  const source = Buffer.from(raw);
  const sections = new Map();
  const walk = (header, body, part) => {
    const h = fields(header);
    const type = params(h['content-type'] || 'text/plain');
    const node = { type: type.value };
    if (part) node.part = part;
    if (Object.keys(type.params).length) node.parameters = type.params;
    if (type.value.startsWith('multipart/')) {
      node.childNodes = children(body, type.params.boundary).map((child, i) => walk(...split(child), part ? `${part}.${i + 1}` : String(i + 1)));
    } else {
      if (h['content-id']) node.id = h['content-id'];
      node.encoding = (h['content-transfer-encoding'] || '7bit').toLowerCase();
      node.size = body.length;
    }
    if (h['content-disposition']) {
      const d = params(h['content-disposition']);
      node.disposition = d.value;
      if (Object.keys(d.params).length) node.dispositionParameters = d.params;
    }
    if (part) {
      sections.set(`${part}.mime`, header);
      sections.set(part, body);
    }
    return node;
  };
  const [header, body] = split(source);
  const structure = walk(header, body, null);
  return { source, header, structure, size: source.length, sections };
}

// An imapflow client for one message, as ImapAccount uses it: fetchOne answers with the fields asked for. log gets
// each fetch's query.
function mimeClient(raw, { uid = 7, log = [] } = {}) {
  const view = serverView(raw);
  return {
    usable: true,
    view,
    getMailboxLock: async () => ({ release() {} }),
    fetchOne: async (seq, query) => {
      log.push(query);
      if (String(seq) !== String(uid)) return false;
      const msg = { uid };
      if (query.source) msg.source = view.source;
      if (query.bodyStructure) msg.bodyStructure = view.structure;
      if (query.headers) msg.headers = view.header;
      if (query.size) msg.size = view.size;
      if (query.bodyParts) {
        msg.bodyParts = new Map();
        for (const key of query.bodyParts) {
          const found = view.sections.get(String(key).toLowerCase());
          if (found) msg.bodyParts.set(String(key).toLowerCase(), found);
        }
      }
      return msg;
    },
    logout: async () => {}
  };
}

module.exports = { serverView, mimeClient };
