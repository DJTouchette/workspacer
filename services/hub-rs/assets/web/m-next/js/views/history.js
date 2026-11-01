// Session history (native ui/recent.rs): every session this hub has ever
// seen — the live fleet plus sessions.history (docs/session-roster.md §5) —
// grouped by project and searchable. Hidden sessions stay here with a badge
// and their own tab; Delete removes a stopped one for good. An older hub's
// sessions.recent still works, without paging or delete.
import { call, can } from '../bus.js';
import {
  sessions, recents, loadRecents, loadOlderHistory, searchHistory, forgetSession,
  historyAvailable, historyNext, historySearch, archiveReady,
  isArchived, setArchived, status, titleOf, modelOf, providerOf, seed,
} from '../store.js';
import { esc, ic, mark, when, basename, modelName, errText } from '../util.js';
import { notice } from '../ui.js';

let tab = 'all';
let query = '';
const SEARCH_DEBOUNCE_MS = 300;

export function mount(root, ctx) {
  root.innerHTML = `<div class="screen page-screen">
    <div class="pagehead"><button class="ibtn" data-back aria-label="Back">${ic('chevron-left', 's20')}</button><span class="grow"></span>
      <button class="ibtn" data-refresh aria-label="Refresh">${ic('refresh-cw', 's18')}</button></div>
    <div class="page scrolly">
      <div class="ptitle"><h1>Session history</h1><p>Every session this hub has seen, hidden ones included.</p></div>
      <label class="search inpage">${ic('search', 's18')}<input type="search" placeholder="Search history…" aria-label="Search history"></label>
      <div class="seg self-start" role="tablist"><button role="tab" data-tab="all">All sessions</button><button role="tab" data-tab="hidden">Hidden</button></div>
      <div data-list></div>
      <div data-more></div>
      <p class="muted small">Hide takes a session off every sidebar on this hub and keeps it here; it never stops an agent. Delete removes a stopped session from history for good.</p>
    </div>
  </div>`;
  const $ = (s) => root.querySelector(s);
  let searchTimer = null;
  let loadingMore = false;
  $('[data-back]').onclick = () => ctx.back();
  $('[data-refresh]').onclick = () => { loadRecents(); seed(); };
  const input = $('input');
  input.value = query;
  input.oninput = () => {
    query = input.value.trim().toLowerCase();
    update();
    clearTimeout(searchTimer);
    if (query && historyAvailable) searchTimer = setTimeout(() => searchHistory(query), SEARCH_DEBOUNCE_MS);
  };
  root.querySelector('.seg').onclick = (e) => { const b = e.target.closest('[data-tab]'); if (b) { tab = b.dataset.tab; update(); } };
  $('[data-more]').onclick = async (e) => {
    if (!e.target.closest('[data-load-more]') || loadingMore) return;
    loadingMore = true; update();
    const err = await loadOlderHistory();
    loadingMore = false; update();
    if (err) notice('Could not load older sessions: ' + err, 'error');
  };
  $('[data-list]').onclick = async (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    if (b.dataset.open) ctx.go('#/s/' + encodeURIComponent(b.dataset.open));
    else if (b.dataset.unhide) {
      const err = await setArchived(b.dataset.unhide, false);
      notice(err ? 'Unhide failed: ' + err : 'Back on the sidebar', err ? 'error' : 'success');
    } else if (b.dataset.forget) {
      const id = b.dataset.forget;
      if (!confirm('Delete this session from history for good? Its saved transcript copy is removed too. This cannot be undone.')) return;
      b.disabled = true;
      const err = await forgetSession(id);
      notice(err ? 'Delete failed: ' + err : 'Deleted from history', err ? 'error' : 'success');
      if (err) update();
    } else if (b.dataset.resume) {
      const r = findRow(b.dataset.resume);
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

  function findRow(id) {
    return (recents || []).find((x) => x && x.sessionId === id)
      || (historySearch ? historySearch.rows.find((x) => x && x.sessionId === id) : null);
  }
  // Hidden = the shared archive once known (with clicks on their way), else
  // what the history row itself said.
  const hidden = (id, row) => isArchived(id) || (!archiveReady && !!(row && row.hidden));

  function rows() {
    const out = new Map();
    for (const s of sessions.values()) {
      const st = status(s);
      out.set(s.sessionId, { id: s.sessionId, title: titleOf(s), cwd: s.cwd, provider: providerOf(s), model: modelOf(s), at: s.lastActivity || 0, st, live: true, hub: s.hub, row: null });
    }
    const add = (r, matched) => {
      if (!r || !r.sessionId) return;
      const have = out.get(r.sessionId);
      if (have) { if (matched) have.matched = true; if (!have.row) have.row = r; return; }
      out.set(r.sessionId, { id: r.sessionId, title: r.title || r.name || basename(r.cwd) || r.sessionId.slice(0, 8), cwd: r.cwd, provider: r.provider || 'claude',
        model: r.model || '', at: r.updatedAt || r.startedAt || 0, st: { label: 'Ended', tone: 'muted' }, live: false, row: r, matched });
    };
    for (const r of recents || []) add(r, false);
    if (query && historySearch && historySearch.query === query) for (const r of historySearch.rows) add(r, true);
    return [...out.values()]
      .filter((r) => tab !== 'hidden' || hidden(r.id, r.row))
      .filter((r) => !query || r.matched || [r.title, r.cwd, r.model, r.provider].some((v) => String(v || '').toLowerCase().includes(query)))
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
    const running = (r) => r.live && !['Ended', 'Paused', 'Stopped'].includes(r.st.label);
    const html = [...groups].map(([name, list]) => `<div class="hgroup"><div class="hh">${ic('folder', 's14 muted')}${esc(name)}<span class="faint">${list.length}</span></div>
      <div class="group">${list.map((r) => {
        const isHidden = hidden(r.id, r.row);
        const actions = [];
        if (isHidden) actions.push(`<button class="btn sm" data-unhide="${esc(r.id)}">Unhide</button>`);
        if (r.live) actions.push(`<button class="btn sm${running(r) ? ' primary' : ''}" data-open="${esc(r.id)}">Open</button>`);
        else actions.push(`<button class="btn sm" data-resume="${esc(r.id)}"${can('agents.spawn') ? '' : ' disabled'}>Resume…</button>`);
        if (historyAvailable && !r.hub) {
          actions.push(`<button class="btn sm danger-text" data-forget="${esc(r.id)}"${running(r) ? ' disabled title="Stop the session before deleting it from history"' : ''}>Delete…</button>`);
        }
        return `<div class="hrow"><div class="tx"><div class="n">${esc(r.title)}${isHidden ? ' <span class="badge muted">Hidden</span>' : ''}</div>
          <div class="m"><span class="dot t-${r.st.tone}"></span><span class="t-${r.st.tone} w5">${esc(r.st.label)}</span><span class="faint">·</span>${mark(r.provider, 's12')}<span>${esc(modelName(r.model) || r.provider)}</span><span class="faint">·</span><span>${esc(when(r.at))}</span></div></div>${actions.join('')}</div>`;
      }).join('')}</div></div>`).join('');
    $('[data-list]').innerHTML = html || `<div class="empty">${ic('history', 's20 muted')}<b>${tab === 'hidden' ? 'Nothing hidden' : query ? 'No matches' : 'No history yet'}</b></div>`;
    $('[data-more]').innerHTML = !query && historyAvailable && historyNext
      ? `<button class="btn sm self-start" data-load-more${loadingMore ? ' disabled' : ''}>${loadingMore ? 'Loading older sessions…' : 'Load older sessions'}</button>`
      : '';
  }
  loadRecents();
  update();
  return { update };
}
