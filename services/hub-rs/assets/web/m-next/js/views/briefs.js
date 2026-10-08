// Briefs (a Settings row now, not a tab): each project's living
// `.workspacer/brief.md`, read-only. Reading a file is operator surface
// (fs.read), scoped by the host to live agents' folders — which is exactly
// the list here. Ported from /m's brief reader.
import { call, can } from '../bus.js';
import { sessions, hubOf } from '../store.js';
import { esc, ic, basename, firstLine, errText } from '../util.js';
import { renderMarkdown } from '../markdown.js';

const FILE = '/.workspacer/brief.md';
const ORDER = ['now', 'direction', 'recently', 'user'];
const rank = (t) => { const i = ORDER.indexOf(String(t || '').trim().toLowerCase()); return i < 0 ? ORDER.length : i; };
const cache = new Map();   // key -> {state, text, sections, err}

function targets() {
  const seen = new Map();
  for (const s of sessions.values()) {
    const cwd = String(s.cwd || '').replace(/\/+$/, '');
    if (!cwd) continue;
    const hub = hubOf(s), key = hub + '|' + cwd;
    const cur = seen.get(key) || { key, cwd, hub, fleet: false, at: 0 };
    cur.fleet = cur.fleet || !!s.isWakeTarget;
    cur.at = Math.max(cur.at, s.lastActivity || 0);
    seen.set(key, cur);
  }
  return [...seen.values()].sort((a, b) => (Number(b.fleet) - Number(a.fleet)) || (b.at - a.at));
}
function parse(text) {
  const out = [];
  let cur = null;
  for (const line of String(text).replace(/\r\n?/g, '\n').split('\n')) {
    const m = /^##\s+(.+?)\s*$/.exec(line);
    if (m) { cur = { title: m[1], body: [] }; out.push(cur); continue; }
    if (/^#\s+/.test(line)) continue;
    if (cur) cur.body.push(line);
  }
  return out.map((s) => ({ title: s.title, body: s.body.join('\n').trim() })).filter((s) => s.body)
    .map((sec, i) => ({ sec, i })).sort((a, b) => (rank(a.sec.title) - rank(b.sec.title)) || (a.i - b.i)).map((x) => x.sec);
}

export function mount(root, ctx) {
  root.innerHTML = `<div class="screen page-screen">
    <div class="pagehead"><button class="ibtn" data-back aria-label="Back">${ic('chevron-left', 's20')}</button><span class="grow"></span></div>
    <div class="page scrolly"><div class="ptitle"><span class="overline">Workspace</span><h1>Briefs</h1>
      <p>Each project keeps a living brief at .workspacer/brief.md — what is underway, where it is going, what was done. Read-only.</p></div>
      <div data-list></div></div>
  </div>`;
  root.querySelector('[data-back]').onclick = () => ctx.back();
  const open = new Set();
  const list = root.querySelector('[data-list]');
  list.onclick = (e) => {
    const b = e.target.closest('[data-brief]');
    if (b) { open.has(b.dataset.brief) ? open.delete(b.dataset.brief) : open.add(b.dataset.brief); update(); }
    const r = e.target.closest('[data-refresh]');
    if (r) { cache.delete(r.dataset.refresh); update(); }
  };
  function fetchBrief(t) {
    if (cache.has(t.key)) return;
    cache.set(t.key, { state: 'loading' });
    call(t.hub ? `hub:${t.hub}/fs.read` : 'fs.read', { path: t.cwd + FILE }, 12000).then((r) => {
      const text = (r && r.contents) || '';
      cache.set(t.key, { state: 'ok', text, sections: parse(text) });
    }).catch((e) => {
      const msg = errText(e);
      cache.set(t.key, { state: /enoent|no such file|not a regular file|not found/i.test(msg) ? 'missing' : 'error', err: msg });
    }).finally(update);
  }
  function update() {
    if (!can('fs.read')) { list.innerHTML = `<div class="empty"><b>Briefs need an operator token</b></div>`; return; }
    const ts = targets();
    if (!ts.length) { list.innerHTML = `<div class="empty">${ic('file-text', 's20 muted')}<b>No projects yet</b></div>`; return; }
    list.innerHTML = ts.map((t) => {
      fetchBrief(t);
      const b = cache.get(t.key) || { state: 'loading' };
      const isOpen = open.has(t.key);
      const now = b.state === 'ok' ? b.sections.find((x) => rank(x.title) === 0) : null;
      const sub = b.state === 'loading' ? 'Reading…' : b.state === 'missing' ? 'Not written yet' : b.state === 'error' ? 'Unreadable'
        : (now && firstLine(now.body.split('\n').find((l) => l.trim()) || '')) || `${b.sections.length} sections`;
      const body = !isOpen ? '' : b.state === 'ok'
        ? `<div class="bbody">${(b.sections.length ? b.sections : [{ title: '', body: b.text }]).map((s) => `${s.title ? `<div class="overline">${esc(s.title)}</div>` : ''}<div class="prose">${renderMarkdown(s.body)}</div>`).join('')}
          <button class="btn sm" data-refresh="${esc(t.key)}">${ic('refresh-cw', 's14')}Refresh</button></div>`
        : `<div class="bbody muted small">${b.state === 'missing' ? `No brief here yet. Ask the Fleet Manager to set up project briefs and it writes one at ${esc(t.cwd + FILE)}.` : b.state === 'error' ? `Couldn’t read it — ${esc(b.err)}` : 'Reading…'}</div>`;
      return `<div class="card brief"><button class="bh" data-brief="${esc(t.key)}" aria-expanded="${isOpen}">${ic('file-text', 's16 t-accent')}
        <span class="tx"><span class="n">${esc(basename(t.cwd))}${t.fleet ? ' <span class="badge accent">Fleet</span>' : ''}${t.hub ? ` <span class="peer">${esc(t.hub)}</span>` : ''}</span><span class="m">${esc(sub)}</span></span>
        ${ic(isOpen ? 'chevron-up' : 'chevron-down', 's16 muted')}</button>${body}</div>`;
    }).join('');
  }
  update();
  return { update };
}
