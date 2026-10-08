// Sessions: native's sidebar as the phone's home screen. Rows nest children
// (sessions this one started, and provider subagents) by depth; the filter
// adds the phone's one difference — "Needs you" — since a phone is opened
// because a push said something needs you.
import { bus, can, reconnectNow } from '../bus.js';
import {
  sessions, archiveReady, isArchived, status, isWorking, isPaused, needsYou, titleOf, projectOf, modelOf, providerOf,
  hubOf, hubDown, peers, coldCache, childrenOf, childActive, childStatus, isCleared, clearChild, setArchived,
  keptSupported, canResume, usage, loadUsage, proposals, seeded,
} from '../store.js';
import { esc, ic, mark, spinner, brand, modelName, resetsIn } from '../util.js';
import { notice } from '../ui.js';
import { pushOk, permission, ensurePush } from '../push.js';
import { nodesHtml, bindNodes } from './nodes.js';

const FILTERS = [
  { id: 'all', label: 'All' },
  { id: 'needs', label: 'Needs you' },
  { id: 'working', label: 'Working' },
  { id: 'paused', label: 'Paused' },
];
let filter = 'all';
let query = '';

const pausedNow = (s) => (keptSupported ? isPaused(s) : canResume(s));
function matches(s) {
  if (filter === 'needs') return needsYou(s);
  if (filter === 'working') return isWorking(s);
  if (filter === 'paused') return pausedNow(s);
  return true;
}
function matchesQuery(s) {
  if (!query) return true;
  const q = query.toLowerCase();
  return [titleOf(s), s.cwd, modelOf(s), hubOf(s), providerOf(s)].some((v) => String(v || '').toLowerCase().includes(q));
}
const childMatches = (c) => {
  if (filter === 'needs') return /approval|question|waiting/.test(c.status || '');
  if (filter === 'working') return childActive(c) && !/approval|question|waiting/.test(c.status || '');
  if (filter === 'paused') return false;
  return true;
};

/** Parent-first tree, as native's session_tree: missing/filtered parents
 *  become roots; a visited set keeps malformed cycles visible. */
function tree(list) {
  const ids = new Map(list.map((s) => [s.sessionId, s]));
  const kids = new Map(), roots = [];
  for (const s of list) {
    const p = s.parentSessionId;
    if (p && p !== s.sessionId && ids.has(p)) { if (!kids.has(p)) kids.set(p, []); kids.get(p).push(s); }
    else roots.push(s);
  }
  const out = [], seen = new Set();
  const visit = (s, depth) => {
    if (seen.has(s.sessionId)) return;
    seen.add(s.sessionId);
    out.push({ s, depth });
    for (const c of childrenOf(s)) {
      const active = childActive(c);
      if (!active && isCleared(s.sessionId, c.id)) continue;
      if (!childMatches(c) && !(filter === 'all')) continue;
      out.push({ child: c, parent: s, depth: depth + 1 });
    }
    for (const k of kids.get(s.sessionId) || []) visit(k, depth + 1);
  };
  for (const r of roots) visit(r, 0);
  for (const s of list) visit(s, 0);
  return out;
}

function sessionRow(s, depth) {
  const st = status(s);
  const cold = coldCache(s);
  const lead = isWorking(s) ? spinner() : st.tone === 'warning' ? '<span class="dot t-warning"></span>' : '';
  const model = modelName(modelOf(s));
  const clay = providerOf(s) === 'claude';
  const peer = hubOf(s) ? `<span class="peer${hubDown(s) ? ' off' : ''}">${ic('server', 's12')}${esc(hubOf(s))}</span>` : '';
  const proj = projectOf(s) && depth === 0 ? `<span class="sep">·</span>${ic('folder', 's12')}<span class="pj">${esc(projectOf(s))}</span>` : '';
  return `<div class="rowwrap" data-swipe data-id="${esc(s.sessionId)}">
    <div class="acts">${can('sessionArchive.set') ? `<button class="arch" data-archive="${esc(s.sessionId)}">${ic('inbox', 's18')}Archive</button>` : ''}</div>
    <button class="row d${Math.min(depth, 3)}" data-open="${esc(s.sessionId)}" aria-label="${esc(titleOf(s))}, ${esc(st.label)}">
      <span class="l1"><span class="title">${esc(titleOf(s))}</span>${cold ? `<span class="cold">${ic('snowflake', 's12')}cold</span>` : ''}${lead}</span>
      <span class="l2">${mark(providerOf(s))}<span class="${clay ? 't-clay' : 'mdl'}">${esc(model || (clay ? 'Claude' : providerOf(s)))}</span>${proj}${peer}
        <span class="st t-${st.tone}">${esc(st.label)}</span></span>
    </button>
  </div>`;
}
function childRow(c, parent, depth) {
  const st = childStatus(c);
  const active = childActive(c);
  const name = c.description || c.type || 'Subagent';
  return `<div class="rowwrap" data-swipe data-id="${esc(parent.sessionId)}/${esc(c.id)}">
    <div class="acts">${!active ? `<button class="clear" data-clear="${esc(parent.sessionId)}" data-child="${esc(c.id)}">${ic('eye-off', 's18')}Clear</button>` : ''}</div>
    <button class="row child d${Math.min(depth, 3)}" data-child-open="${esc(parent.sessionId)}" data-agent="${esc(c.id)}" aria-label="${esc(name)}, ${esc(st.label)}">
      <span class="l1"><span class="title">${esc(name)}</span>${st.label === 'Working' ? spinner() : st.tone === 'warning' ? '<span class="dot t-warning"></span>' : ''}</span>
      <span class="l2">${ic('bot', 's12')}<span>${esc(c.type || 'Subagent')}</span>${c.model ? `<span class="sep">·</span><span>${esc(modelName(c.model))}</span>` : ''}
        <span class="st t-${st.tone}">${esc(st.label)}</span></span>
    </button>
  </div>`;
}

// ── usage meters (native usage.rs `accounts`) ────────────────────────────
const WINDOWS = [['five_hour', '5h'], ['seven_day', 'Week'], ['monthly', 'Month']];
function accounts(report) {
  const out = [];
  const now = Math.floor(Date.now() / 1000);
  for (const p of (report && report.providers) || []) {
    for (const a of p.accounts || []) {
      if (a.source === 'transcript' || a.label === 'unattributed') continue;
      const windows = [];
      for (const [key, label] of WINDOWS) {
        const w = a.windows && a.windows[key];
        if (!w || (w.used_percent && w.used_percent.state === 'unavailable')) continue;
        const resets = w.resets_at > 0 ? w.resets_at : null;
        if (w.is_current === false || (resets && resets <= now)) continue;
        const pct = w.used_percent && w.used_percent.state === 'ok' && resets ? w.used_percent.value : null;
        if (typeof pct !== 'number' || !Number.isFinite(pct) || pct < 0 || pct > 100) continue;
        const pace = w.pace && w.pace.known === true ? w.pace : null;
        windows.push({ label, pct, resets, expected: pace && typeof pace.expectedPct === 'number' ? pace.expectedPct : null });
      }
      const failure = a.failure && a.failure.detail;
      let unmeasured = null;
      if (!windows.length) {
        const states = WINDOWS.map(([k]) => a.windows && a.windows[k] && a.windows[k].used_percent).filter(Boolean);
        if (!failure && states.every((w) => w.state === 'unavailable')) continue;
        const reauth = (a.failure && a.failure.kind === 'needs_reauth');
        unmeasured = reauth ? 'Sign in again' : failure ? 'Refresh failed' : 'No reading yet';
      }
      const name = { claude: 'Claude', codex: 'Codex', copilot: 'Copilot' }[p.provider] || p.provider;
      const title = a.is_default || !a.label || a.label === 'default' ? name : `${name} · ${a.label}`;
      out.push({ provider: p.provider, title, windows, unmeasured, stale: a.fresh === false });
    }
  }
  return out;
}
function usageHtml() {
  const list = accounts(usage);
  if (!list.length) return '';
  const rows = list.map((a) => {
    const w = a.windows[0];
    const tone = w ? (w.pct >= 90 ? 'error' : w.pct >= 70 ? 'warning' : 'success') : '';
    return `<div class="u">
      <span class="nm">${mark(a.provider)}<span class="${a.provider === 'claude' ? 't-clay' : ''}">${esc(a.title)}</span></span>
      ${w ? `<span class="muted" title="Resets in ${esc(resetsIn(w.resets))}">${esc(w.label)}</span>
        <span class="meter"><i style="width:${w.pct}%;background:var(--wks-${tone})"></i>${w.expected != null ? `<b style="left:${Math.min(100, w.expected)}%"></b>` : ''}</span>
        <span class="t-${tone} pct">${Math.round(w.pct)}%</span>`
        : `<span class="${a.unmeasured === 'No reading yet' ? 'muted' : 't-error'}">${esc(a.unmeasured)}</span>`}
    </div>`;
  }).join('');
  return `<div class="card usage" aria-label="Usage">${rows}</div>`;
}

function hubName() {
  const h = location.hostname;
  return /^[\d.]+$|^\[|^localhost$/.test(h) ? h : h.split('.')[0];
}

export function mount(root, ctx) {
  root.innerHTML = `<div class="screen sessions">
    <header class="top">${brand}
      <button class="hubline" data-hub></button>
      <span class="grow"></span>
      <button class="ibtn" data-go="#/projects" aria-label="Projects">${ic('folder', 's18')}</button>
      <button class="ibtn" data-go="#/history" aria-label="History">${ic('book-open', 's18')}</button>
      <button class="ibtn" data-go="#/jobs" aria-label="Jobs" data-jobs>${ic('calendar', 's18')}</button>
      <button class="ibtn" data-go="#/settings" aria-label="Settings">${ic('settings', 's18')}</button>
    </header>
    <label class="search">${ic('search', 's18')}<input type="search" placeholder="Search sessions…" aria-label="Search sessions" autocomplete="off" enterkeyhint="search"></label>
    <div class="segrow"><div class="seg" role="tablist" aria-label="Filter sessions"></div></div>
    <div class="list scrolly" data-list></div>
    <div class="fade-b"></div>
    <button class="fab" data-go="#/new">${ic('plus', 's18')}New agent</button>
  </div>`;
  const $ = (sel) => root.querySelector(sel);
  const input = $('input');
  input.value = query;
  input.oninput = () => { query = input.value.trim(); update(); };
  for (const b of root.querySelectorAll('[data-go]')) b.onclick = () => ctx.go(b.dataset.go);
  $('[data-hub]').onclick = () => { if (!bus.connected) { reconnectNow(); notice('Reconnecting…'); } else ctx.go('#/settings'); };
  const list = $('[data-list]');
  let openSwipe = null;

  list.addEventListener('click', async (e) => {
    const t = e.target.closest('button');
    if (!t) return;
    if (openSwipe && !t.closest('.acts')) {
      const was = openSwipe; closeSwipe();
      if (t.closest('.rowwrap') === was) return;
    }
    if (t.dataset.open) ctx.go('#/s/' + encodeURIComponent(t.dataset.open));
    else if (t.dataset.childOpen) ctx.go(`#/s/${encodeURIComponent(t.dataset.childOpen)}/a/${encodeURIComponent(t.dataset.agent)}`);
    else if (t.dataset.archive) {
      closeSwipe();
      const err = await setArchived(t.dataset.archive, true);
      notice(err ? 'Archive failed: ' + err : 'Archived — find it in History', err ? 'error' : '');
    } else if (t.dataset.clear) { closeSwipe(); clearChild(t.dataset.clear, t.dataset.child); }
    else if (t.dataset.push) { notice(await ensurePush(false) || 'Notifications on'); update(); }
    else if (t.dataset.filter) { filter = t.dataset.filter; update(); }
  });
  root.querySelector('.seg').addEventListener('click', (e) => {
    const b = e.target.closest('[data-filter]');
    if (b) { filter = b.dataset.filter; update(); list.scrollTop = 0; }
  });
  bindNodes(list);

  // ── swipe → Clear / Archive (native's row hover actions by name) ──────
  function closeSwipe() {
    if (!openSwipe) return;
    const row = openSwipe.querySelector('.row');
    row.style.transform = '';
    openSwipe.classList.remove('open');
    openSwipe = null;
  }
  let drag = null;
  list.addEventListener('pointerdown', (e) => {
    const wrap = e.target.closest('[data-swipe]');
    if (!wrap || e.target.closest('.acts')) return;
    drag = { wrap, x: e.clientX, y: e.clientY, dx: 0, active: false, width: wrap.querySelector('.acts').offsetWidth };
  });
  list.addEventListener('pointermove', (e) => {
    if (!drag) return;
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
    if (!drag.active) {
      if (Math.abs(dx) < 10 || Math.abs(dx) < Math.abs(dy) * 1.2 || !drag.width) { if (Math.abs(dy) > 12) drag = null; return; }
      drag.active = true;
      if (openSwipe && openSwipe !== drag.wrap) closeSwipe();
      drag.wrap.classList.add('dragging');
    }
    const base = drag.wrap.classList.contains('open') ? -drag.width : 0;
    drag.dx = Math.max(-drag.width - 24, Math.min(0, base + dx));
    drag.wrap.querySelector('.row').style.transform = `translateX(${drag.dx}px)`;
  });
  const end = () => {
    if (!drag) return;
    const d = drag; drag = null;
    if (!d.active) return;
    d.wrap.classList.remove('dragging');
    const row = d.wrap.querySelector('.row');
    if (d.dx < -d.width / 2) { row.style.transform = `translateX(${-d.width}px)`; d.wrap.classList.add('open'); openSwipe = d.wrap; }
    else { row.style.transform = ''; d.wrap.classList.remove('open'); if (openSwipe === d.wrap) openSwipe = null; }
    // Swallow the click that ends a drag.
    d.wrap.addEventListener('click', (ev) => { ev.stopPropagation(); ev.preventDefault(); }, { capture: true, once: true });
  };
  list.addEventListener('pointerup', end);
  list.addEventListener('pointercancel', end);

  function update() {
    // header
    const hub = $('[data-hub]');
    const n = peers.length;
    hub.innerHTML = bus.connected
      ? `<span class="dot t-success"></span>${esc(hubName())}${n ? `<span class="faint">· +${n} peer${n === 1 ? '' : 's'}</span>` : ''}`
      : `<span class="dot t-warning"></span><span class="t-warning">Reconnecting…</span>`;
    const jb = $('[data-jobs]');
    jb.hidden = !can('jobs.list');
    jb.classList.toggle('pip', proposals().length > 0);

    const all = [...sessions.values()].filter((s) => !isArchived(s.sessionId));
    const counts = {
      all: all.length,
      needs: all.filter(needsYou).length,
      working: all.filter(isWorking).length,
      paused: all.filter(pausedNow).length,
    };
    root.querySelector('.seg').innerHTML = FILTERS.map((f) =>
      `<button role="tab" aria-selected="${filter === f.id}" class="${filter === f.id ? 'on' : ''}" data-filter="${f.id}">` +
      `${f.id === 'needs' && counts.needs ? '<span class="dot t-warning"></span>' : ''}${esc(f.label)} <span class="n">${counts[f.id]}</span></button>`).join('');

    const shown = all.filter((s) => matches(s) && matchesQuery(s));
    // Child rows whose parent matched the query still show.
    const rows = tree(shown);
    const parts = [nodesHtml()];
    if (pushOk && permission() === 'default' && bus.connected) {
      parts.push(`<div class="card banner">${ic('bell', 's18 t-accent')}<div class="tx"><b>Get a notification when an agent needs you</b><span>Approvals and questions reach this phone even when the app is closed.</span></div><button class="btn sm primary" data-push>Turn on</button></div>`);
    }
    if (!archiveReady && sessions.size) {
      parts.push(`<div class="empty">${spinner()}<p>Loading sessions…</p></div>`);
    } else if (!rows.length) {
      const msg = !bus.connected && !seeded ? ['Connecting to your workspace', 'Sessions appear here once the hub answers.']
        : query ? ['No matching sessions', 'Try a different search.']
        : filter === 'needs' ? ['Nothing needs you', 'Approvals and questions show up here, and as notifications.']
        : filter === 'working' ? ['Nothing is working', 'Agents that are mid-turn show up here.']
        : filter === 'paused' ? ['No paused sessions', keptSupported ? 'Sessions open when the desktop closed show up here.' : 'Stopped sessions you can pick back up show up here.']
        : ['No sessions yet', 'Start an agent with the button below.'];
      parts.push(`<div class="empty">${ic(filter === 'needs' ? 'circle-check' : 'inbox', 's20 muted')}<b>${esc(msg[0])}</b><p>${esc(msg[1])}</p></div>`);
    } else {
      parts.push(`<div class="secthead"><span class="overline">${filter === 'all' ? 'Sessions' : esc(FILTERS.find((f) => f.id === filter).label)}</span><span class="overline">${shown.length}</span></div>`);
      parts.push(rows.map((r) => (r.child ? childRow(r.child, r.parent, r.depth) : sessionRow(r.s, r.depth))).join(''));
    }
    parts.push(usageHtml());
    const prevTop = list.scrollTop;
    const keepOpen = openSwipe && openSwipe.dataset.id;
    openSwipe = null;
    list.innerHTML = parts.join('');
    list.scrollTop = prevTop;
    if (keepOpen) {
      const w = [...list.querySelectorAll('[data-swipe]')].find((x) => x.dataset.id === keepOpen);
      if (w) { w.classList.add('open'); w.querySelector('.row').style.transform = `translateX(${-w.querySelector('.acts').offsetWidth}px)`; openSwipe = w; }
    }
  }
  loadUsage();
  const usageTimer = setInterval(() => { if (document.visibilityState === 'visible') loadUsage(); }, 60000);
  // Clock-driven state (cache going cold, elapsed) redraws on a slow tick.
  const tick = setInterval(() => { if (document.visibilityState === 'visible' && !drag) update(); }, 15000);
  update();
  return { update, destroy() { clearInterval(usageTimer); clearInterval(tick); } };
}

