// Response cards: an agent's ```wks-html-card fence (see the
// workspacer-response-cards skill). Parsed and validated exactly as native's
// `parse_card` (transcript.rs): title + fallback are required, actions only
// count on a v1 envelope with a body, at most 8, each with a short label.
//
// The body is agent-authored HTML, so it goes through an allowlist: text,
// structure and tables survive; scripts, styles, forms, images (no remote
// loads), event handlers and every attribute but a safe link href do not.
// The hub's CSP on /m-next is the second wall.
import { esc, ic } from './util.js';

const ACTION_LIMITS = { open_worker: ['sessionId', 200], view_diff: ['path', 4096], fill_composer: ['text', 4000] };

export function parseCard(raw) {
  if (raw.length > 65536) return null;
  let v;
  try { v = JSON.parse(raw); } catch { return null; }
  if (!v || typeof v !== 'object') return null;
  const title = typeof v.title === 'string' ? v.title.trim() : '';
  const fallback = typeof v.fallback === 'string' ? v.fallback.trim() : '';
  if (!title || [...title].length > 120 || !fallback || [...fallback].length > 4000) return null;
  const envelope = v.v === 1 && typeof v.bodyHtml === 'string' && v.bodyHtml.trim() &&
    (v.css === undefined || (typeof v.css === 'string' && v.css.length <= 16384));
  let actions = [];
  if (envelope && Array.isArray(v.actions ?? [])) {
    const list = v.actions || [];
    const valid = list.length <= 8 && list.every((a) => {
      const spec = a && ACTION_LIMITS[a.kind];
      if (!spec) return false;
      const value = a[spec[0]];
      return typeof a.label === 'string' && a.label.trim() && [...a.label].length <= 48 &&
        typeof value === 'string' && value.trim() && [...value].length <= spec[1];
    });
    if (valid) actions = list;
  }
  return { title, fallback, body: v.v === 1 && typeof v.bodyHtml === 'string' ? sanitize(v.bodyHtml) : '', actions };
}

const KEEP = new Set(['p', 'div', 'span', 'section', 'header', 'footer', 'b', 'strong', 'i', 'em', 'u', 's', 'del', 'small', 'sub', 'sup',
  'code', 'pre', 'kbd', 'ul', 'ol', 'li', 'dl', 'dt', 'dd', 'table', 'thead', 'tbody', 'tfoot', 'tr', 'th', 'td', 'caption',
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'br', 'hr', 'blockquote', 'a', 'details', 'summary', 'mark']);
const DROP = new Set(['script', 'style', 'template', 'iframe', 'object', 'embed', 'link', 'meta', 'form', 'input', 'button',
  'select', 'textarea', 'svg', 'math', 'img', 'video', 'audio', 'source', 'picture', 'canvas', 'noscript', 'title', 'head', 'base']);

/** Allowlist HTML → inert HTML string. */
export function sanitize(html) {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const out = document.createElement('div');
  const walk = (src, dst, depth) => {
    if (depth > 24) return;
    for (const node of src.childNodes) {
      if (node.nodeType === Node.TEXT_NODE) { dst.appendChild(document.createTextNode(node.nodeValue)); continue; }
      if (node.nodeType !== Node.ELEMENT_NODE) continue;
      const tag = node.tagName.toLowerCase();
      if (DROP.has(tag)) continue;
      if (!KEEP.has(tag)) { walk(node, dst, depth + 1); continue; }
      const el = document.createElement(tag);
      if (tag === 'a') {
        const href = (node.getAttribute('href') || '').trim();
        if (/^(https?:|mailto:)/i.test(href)) { el.setAttribute('href', href); el.setAttribute('target', '_blank'); el.setAttribute('rel', 'noopener noreferrer'); }
      }
      if ((tag === 'td' || tag === 'th') && /^\d{1,2}$/.test(node.getAttribute('colspan') || '')) el.setAttribute('colspan', node.getAttribute('colspan'));
      walk(node, el, depth + 1);
      dst.appendChild(el);
    }
  };
  walk(doc.body, out, 0);
  return out.innerHTML;
}

const ACTION_META = {
  fill_composer: { icon: 'message-square-plus', prefix: 'Prefill: ' },
  view_diff: { icon: 'file-diff', prefix: 'View diff: ' },
  open_worker: { icon: 'user-round', prefix: 'Open worker: ' },
};

/** The card face. Actions carry their payload in data attributes; the chat
 *  view routes them (fill composer never sends). */
export function cardHtml(card) {
  const body = card.body
    ? `<div class="cbody">${card.body}</div>`
    : `<div class="prose">${esc(card.fallback)}</div>`;
  const acts = card.actions.map((a, i) => {
    const m = ACTION_META[a.kind];
    return `<button class="chipbtn" data-card-action="${i}" data-kind="${esc(a.kind)}" data-value="${esc(a.sessionId || a.path || a.text)}">${ic(m.icon, 's14')}${esc(m.prefix + a.label)}</button>`;
  }).join('');
  return `<div class="rcard"><div class="t">${esc(card.title)}</div>${body}${acts ? `<div class="acts">${acts}</div>` : ''}</div>`;
}
