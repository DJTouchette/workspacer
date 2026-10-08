// Session history (native ui/recent.rs): every session this hub knows about —
// the live fleet plus sessions.recent — grouped by project, searchable, with
// the shared archive as its own tab. Open a known session; Resume… one the
// fleet no longer lists.
import { call, can } from '../bus.js';
import { sessions, recents, loadRecents, isArchived, setArchived, status, titleOf, modelOf, providerOf, seed } from '../store.js';
import { esc, ic, mark, when, basename, modelName, errText } from '../util.js';
import { notice } from '../ui.js';

let tab = 'all';
let query = '';

export function mount(root, ctx) {
  root.innerHTML = `<div class="screen page-screen">
    <div class="pagehead"><button class="ibtn" data-back aria-label="Back">${ic('chevron-left', 's20')}</button><span class="grow"></span>
      <button class="ibtn" data-refresh aria-label="Refresh">${ic('refresh-cw', 's18')}</button></div>
    <div class="page scrolly">
      <div class="ptitle"><h1>Session history</h1><p>Every session this hub knows about, including ended ones.</p></div>
      <label class="search inpage">${ic('search', 's18')}<input type="search" placeholder="Search history…" aria-label="Search history"></label>
      <div class="seg self-start" role="tablist"><button role="tab" data-tab="all">All sessions</button><button role="tab" data-tab="archived">Archived</button></div>
      <div data-list></div>
      <p class="muted small">Archives are shared with every client of this hub. Archiving keeps the conversation and does not stop an agent.</p>
    </div>
  </div>`;
  const $ = (s) => root.querySelector(s);
  $('[data-back]').onclick = () => ctx.back();
  $('[data-refresh]').onclick = () => { loadRecents(); seed(); };
  const input = $('input');
  input.value = query;
  input.oninput = () => { query = input.value.trim().toLowerCase(); update(); };
  root.querySelector('.seg').onclick = (e) => { const b = e.target.closest('[data-tab]'); if (b) { tab = b.dataset.tab; update(); } };
  $('[data-list]').onclick = async (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    if (b.dataset.open) ctx.go('#/s/' + encodeURIComponent(b.dataset.open));
    else if (b.dataset.restore) {
      const err = await setArchived(b.dataset.restore, false);
      notice(err ? 'Restore failed: ' + err : 'Restored', err ? 'error' : 'success');
    } else if (b.dataset.resume) {
      const r = recents.find((x) => x.sessionId === b.dataset.resume);
      if (!r) return;
      if (!can('agents.spawn')) { notice('Resuming needs an operator token', 'warning'); return; }
      b.disabled = true; b.textContent = 'Resuming…';
      try {
        const out = await call('agents.spawn', { cwd: r.cwd, provider: r.provider || 'claude', transport: 'stream', resumeSessionId: r.sessionId });
        await seed();
        const id = (out && out.sessionId) || r.sessionId;
        ctx.go('#/s/' + encodeURIComponent(id));
      } catch (err) { notice('Could not resume: ' + errText(err), 'error'); update(); }
    }
  };

  function rows() {
    const out = new Map();
    for (const s of sessions.values()) {
      const st = status(s);
      out.set(s.sessionId, { id: s.sessionId, title: titleOf(s), cwd: s.cwd, provider: providerOf(s), model: modelOf(s), at: s.lastActivity || 0, st, live: true, hub: s.hub });
    }
    for (const r of recents || []) {
      if (!r || !r.sessionId || out.has(r.sessionId)) continue;
      out.set(r.sessionId, { id: r.sessionId, title: r.title || r.name || basename(r.cwd) || r.sessionId.slice(0, 8), cwd: r.cwd, provider: r.provider || 'claude',
        model: r.model || '', at: r.updatedAt || r.startedAt || 0, st: { label: 'Ended', tone: 'muted' }, live: false });
    }
    return [...out.values()]
      .filter((r) => isArchived(r.id) === (tab === 'archived'))
      .filter((r) => !query || [r.title, r.cwd, r.model, r.provider].some((v) => String(v || '').toLowerCase().includes(query)))
      .sort((a, b) => b.at - a.at);
  }
  function update() {
    for (const b of root.querySelectorAll('[data-tab]')) { b.classList.toggle('on', b.dataset.tab === tab); b.setAttribute('aria-selected', b.dataset.tab === tab); }
    const groups = new Map();
    for (const r of rows()) {
      const key = r.hub ? `${r.hub} · ${basename(r.cwd)}` : basename(r.cwd) || 'Other';
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(r);
    }
    const html = [...groups].map(([name, list]) => `<div class="hgroup"><div class="hh">${ic('folder', 's14 muted')}${esc(name)}<span class="faint">${list.length}</span></div>
      <div class="group">${list.map((r) => {
        const action = tab === 'archived'
          ? `<button class="btn sm" data-restore="${esc(r.id)}">Restore</button>`
          : r.live ? `<button class="btn sm${r.st.label === 'Ended' || r.st.label === 'Paused' ? '' : ' primary'}" data-open="${esc(r.id)}">Open</button>`
          : `<button class="btn sm" data-resume="${esc(r.id)}"${can('agents.spawn') ? '' : ' disabled'}>Resume…</button>`;
        return `<div class="hrow"><div class="tx"><div class="n">${esc(r.title)}</div>
          <div class="m"><span class="dot t-${r.st.tone}"></span><span class="t-${r.st.tone} w5">${esc(r.st.label)}</span><span class="faint">·</span>${mark(r.provider, 's12')}<span>${esc(modelName(r.model) || r.provider)}</span><span class="faint">·</span><span>${esc(when(r.at))}</span></div></div>${action}</div>`;
      }).join('')}</div></div>`).join('');
    $('[data-list]').innerHTML = html || `<div class="empty">${ic('history', 's20 muted')}<b>${tab === 'archived' ? 'Nothing archived' : query ? 'No matches' : 'No history yet'}</b></div>`;
  }
  loadRecents();
  update();
  return { update };
}
