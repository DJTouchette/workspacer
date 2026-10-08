// The title island, expanded: the phone's session menu (native island.rs
// reveal + features.rs Session details). Also the sheets it opens: model,
// effort and access pickers, "Continue with…" / "Start fresh from a
// summary" (native handoff.rs), Changes and a file's diff.
import { call, can, isOperator, bus } from '../bus.js';
import {
  sessions, status, isLive, isWorking, titleOf, modelOf, effortOf, providerOf, permissionOf, context, gaugeTone,
  costOf, turnsOf, runningTasks, tasksOf, childrenOf, childStatus, isArchived, setArchived, hubDown, qualify, drafts,
} from '../store.js';
import { esc, ic, mark, fmtTokens, fmtUSD, fmtElapsed, modelName, errText, providerName } from '../util.js';
import * as act from '../actions.js';
import { notice, ask, pick, sheet, closeSheet } from '../ui.js';
import { openTasks } from './tasks.js';

const overlay = () => document.getElementById('overlay');

export function openMenu(s, { go }) {
  closeSheet();
  const id = s.sessionId;
  const root = overlay();
  const render = () => {
    const cur = sessions.get(id) || s;
    const st = status(cur);
    const c = context(cur);
    const live = isLive(cur), working = isWorking(cur), op = isOperator() && !hubDown(cur);
    const turns = turnsOf(cur);
    const userTurns = turns.filter((t) => t.role === 'user').length;
    const first = turns.find((t) => t.timestamp);
    const last = [...turns].reverse().find((t) => t.timestamp);
    const tasks = tasksOf(cur), kids = childrenOf(cur);
    const model = modelName(modelOf(cur)) || 'Provider default';
    const provider = providerOf(cur);
    const other = provider === 'codex' ? 'claude' : 'codex';
    const handoffOk = ['claude', 'codex'].includes(provider) && !cur.isWakeTarget && can('agents.spawn') && !!cur.cwd;
    const cost = costOf(cur);
    return `<div class="scrim" data-x></div>
    <div class="islandx" role="dialog" aria-modal="true" aria-label="Session menu">
      <div class="ixh"><span class="dot t-${working ? 'busy' : st.tone}"></span><span class="tt">${esc(titleOf(cur))}</span>
        <span class="badge ${st.tone === 'muted' ? 'neutral' : st.tone}">${esc(st.label)}</span><button class="ibtn sm" data-x aria-label="Close">${ic('x', 's18')}</button></div>
      <div class="ctx">
        <div class="cl muted"><span>Context · ${esc(model)}</span><span class="r">${c ? `<b>${Math.round(c.pct)}%</b>${c.tokens && c.window ? ` · ${esc(fmtTokens(c.tokens))} / ${esc(fmtTokens(c.window))}` : ''}` : 'unknown'}</span></div>
        <div class="ctxbar">${c ? `<i style="width:${Math.min(100, c.pct)}%;background:var(--wks-${gaugeTone(c.pct)})"></i>` : ''}</div>
        <div class="path muted">${ic('folder', 's12')}<span class="mono">${esc(cur.cwd || '—')}</span>${cur.hub ? `<span class="faint">·</span>${ic('server', 's12')}${esc(cur.hub)}` : ''}</div>
      </div>
      <div class="stats"><div><div class="k">Cost</div><div class="v">${cost != null ? esc(fmtUSD(cost)) : '—'}</div></div>
        <div><div class="k">Turns</div><div class="v">${userTurns || '—'}</div></div>
        <div><div class="k">Time</div><div class="v">${first && last && last.timestamp > first.timestamp ? esc(fmtElapsed(last.timestamp - first.timestamp).replace(/ \d+s$/, '')) : '—'}</div></div></div>
      <div class="qa">
        <button data-tasks${tasks.length ? '' : ' disabled'}>${ic('square-terminal', 's18')}${runningTasks(cur) ? `${runningTasks(cur)} running` : `${tasks.length} task${tasks.length === 1 ? '' : 's'}`}</button>
        <button data-children${kids.length ? '' : ' disabled'}>${ic('bot', 's18')}${kids.length} child${kids.length === 1 ? '' : 'ren'}</button>
        <button data-changes>${ic('file-diff', 's18')}Changes</button>
        ${working && can('claude.signal') ? `<button data-interrupt>${ic('square', 's18')}Interrupt</button>` : `<button data-copy-path>${ic('copy', 's18')}Copy path</button>`}
      </div>
      <div class="group menu">
        ${live && op ? `<button class="mi" data-model-pick>${ic('sparkles')}Model<span class="v">${esc(model)} ${ic('chevron-right', 's14')}</span></button>` : ''}
        ${live && op && act.effortsFor(provider).length ? `<button class="mi" data-effort>${ic('zap')}Effort<span class="v">${esc(effortOf(cur) ? effortOf(cur).charAt(0).toUpperCase() + effortOf(cur).slice(1) : 'Default')} ${ic('chevron-right', 's14')}</span></button>` : ''}
        ${live && op ? `<button class="mi" data-access>${ic('shield-check')}Access<span class="v">${esc(act.accessLabel(permissionOf(cur)))} ${ic('chevron-right', 's14')}</span></button>` : ''}
        ${handoffOk ? `<button class="mi" data-handoff>${ic('reply')}Continue with ${esc(providerName(other))}…<span class="v">${ic('chevron-right', 's14')}</span></button>
          <button class="mi" data-fresh>${ic('sparkles')}Start fresh from a summary<span class="v">${ic('chevron-right', 's14')}</span></button>` : ''}
        ${can('sessionArchive.set') ? `<button class="mi" data-archive>${ic('inbox')}${isArchived(id) ? 'Restore from archive' : 'Archive'}</button>` : ''}
        ${live && can('claude.signal') && !hubDown(cur) ? `<button class="mi danger" data-end>${ic('circle-stop')}End session…</button>` : ''}
      </div>
    </div>`;
  };
  root.innerHTML = render();
  root.hidden = false;
  requestAnimationFrame(() => root.classList.add('show', 'menu'));
  const close = () => { root.classList.remove('show', 'menu'); root.hidden = true; root.innerHTML = ''; root.onclick = null; clearInterval(timer); };
  const timer = setInterval(() => { if (!root.hidden && root.querySelector('.islandx')) root.innerHTML = render(); }, 2000);
  root.onclick = async (e) => {
    const b = e.target.closest('[data-x], button');
    if (!b) return;
    const cur = sessions.get(id) || s;
    if (b.hasAttribute('data-x')) { close(); return; }
    if (b.hasAttribute('data-tasks')) { close(); openTasks(cur, { go }); return; }
    if (b.hasAttribute('data-children')) { close(); openChildren(cur, go); return; }
    if (b.hasAttribute('data-changes')) { close(); openChanges(cur); return; }
    if (b.hasAttribute('data-interrupt')) { close(); act.interrupt(id); return; }
    if (b.hasAttribute('data-copy-path')) { close(); try { await navigator.clipboard.writeText(cur.cwd || ''); notice('Copied'); } catch { notice('Copy failed', 'error'); } return; }
    if (b.hasAttribute('data-model-pick')) { close(); openModelPicker(cur); return; }
    if (b.hasAttribute('data-effort')) {
      close();
      const v = await pick('Effort', act.effortsFor(providerOf(cur)).map((x) => ({ value: x, label: x.charAt(0).toUpperCase() + x.slice(1), on: effortOf(cur) === x })));
      if (v) act.setEffort(id, v);
      return;
    }
    if (b.hasAttribute('data-access')) {
      close();
      const modes = act.modesFor(providerOf(cur) === 'claude' ? 'claude' : 'other');
      const v = await pick('Access', modes.map((m) => ({
        value: m.id, label: m.label, detail: m.detail, on: permissionOf(cur) === m.id,
        disabled: act.fullAccess(m.id) && !bus.scope.fullAccess,
      })));
      if (v) act.setAccess(id, v);
      return;
    }
    if (b.hasAttribute('data-handoff')) { close(); openHandoff(cur, { fresh: false, go }); return; }
    if (b.hasAttribute('data-fresh')) { close(); openHandoff(cur, { fresh: true, go }); return; }
    if (b.hasAttribute('data-archive')) {
      close();
      const on = !isArchived(id);
      const err = await setArchived(id, on);
      notice(err ? 'Failed: ' + err : on ? 'Archived — it stays in History' : 'Restored', err ? 'error' : '');
      return;
    }
    if (b.hasAttribute('data-end')) {
      close();
      if (await ask({ title: 'End this session?', body: 'Unlike Interrupt, this ends the agent’s process — the current turn is lost. The conversation stays, and you can resume it later.', confirm: 'End session', danger: true })) act.endSession(id);
    }
  };
  return close;
}

export async function openModelPicker(s) {
  if (!isLive(s) || !isOperator() || !act.canSwitch()) {
    notice(isLive(s) ? 'This token cannot switch models' : 'Resume the session to change its model', 'warning');
    return;
  }
  const provider = providerOf(s);
  const close = sheet(`<div class="sh"><h3>Model</h3></div><div class="empty"><p>Loading models…</p></div>`, { label: 'Model' });
  let models;
  try { models = await act.listModels(provider === 'claude' ? 'claude' : provider, s.cwd, s.sessionId); }
  catch (e) { close(); notice('Could not load models: ' + errText(e), 'error'); return; }
  const current = modelOf(s);
  const v = await pick('Model', models.map((m) => ({ value: m, label: m.label, detail: m.contextWindow ? `${fmtTokens(m.contextWindow)} context` : '', on: m.id === current || m.legacy === current })));
  if (v) act.setModel(s.sessionId, v.legacy, v.id, v.contextWindow);
}

function openChildren(s, go) {
  const kids = childrenOf(s);
  sheet(`<div class="sh"><h3>Subagents</h3><button class="ibtn" data-close aria-label="Close">${ic('x', 's18')}</button></div>
    <div class="group tasklist scrolly">${kids.map((c) => {
      const st = childStatus(c);
      return `<button class="tk" data-agent="${esc(c.id)}">${ic('bot', `s18 t-${st.tone}`)}<span class="tx"><span class="n">${esc(c.description || c.type || 'Subagent')}</span><span class="m">${esc([c.type, st.label].filter(Boolean).join(' · '))}</span></span>${ic('arrow-right', 's16 muted')}</button>`;
    }).join('') || '<div class="empty"><p>No subagents.</p></div>'}</div>`, {
    label: 'Subagents',
    bind(el, close) {
      for (const b of el.querySelectorAll('[data-agent]')) b.onclick = () => { close(); go(`#/s/${encodeURIComponent(s.sessionId)}/a/${encodeURIComponent(b.dataset.agent)}`); };
    },
  });
}

/** Changes: git.status when this token may read it, else the transcript's estimate. */
export async function openChanges(s, estimate) {
  const strip = (p) => { const c = String(s.cwd || '').replace(/\/+$/, ''); return c && p.startsWith(c + '/') ? p.slice(c.length + 1) : p; };
  let rows = estimate ? [...estimate.entries()].map(([p, e]) => ({ path: strip(p), added: e.added, removed: e.removed })) : [];
  let source = estimate ? 'Estimated from the tools this turn used' : '';
  if (!estimate && can('git.status') && s.cwd) {
    try {
      const st = await call(qualify(s.sessionId, 'git.status'), { cwd: s.cwd });
      const files = (st && (st.files || st.entries)) || [];
      rows = files.map((f) => ({ path: f.path || String(f), status: [f.staged && `staged ${f.staged}`, f.unstaged && `${f.unstaged}`].filter(Boolean).join(' · ') || 'changed' }));
      source = st && st.branch ? `On ${st.branch}` : 'Uncommitted changes in this folder';
    } catch (e) { source = 'Could not read the folder: ' + errText(e); }
  } else if (!estimate) source = 'Reading changes needs an operator token';
  sheet(`<div class="sh"><h3>Changes</h3><button class="ibtn" data-close aria-label="Close">${ic('x', 's18')}</button></div>
    <div class="muted small">${esc(source)}</div>
    <div class="group tasklist scrolly">${rows.map((r, i) => `<button class="tk" data-i="${i}">${ic('file-diff', 's16 t-accent')}<span class="tx"><span class="n mono">${esc(r.path)}</span>
      <span class="m">${r.status ? esc(r.status) : `<span class="t-success">+${r.added}</span> <span class="t-error">−${r.removed}</span>`}</span></span>${can('git.diff') ? ic('chevron-right', 's16 muted') : ''}</button>`).join('') || '<div class="empty"><p>No changes.</p></div>'}</div>`, {
    label: 'Changes',
    bind(el, close) {
      if (!can('git.diff')) return;
      for (const b of el.querySelectorAll('[data-i]')) b.onclick = () => { close(); openDiff(s, rows[Number(b.dataset.i)].path); };
    },
  });
}

export async function openDiff(s, path) {
  if (!can('git.diff')) { notice('Viewing a diff needs an operator token', 'warning'); return; }
  sheet(`<div class="sh"><h3 class="mono small">${esc(path)}</h3><button class="ibtn" data-close aria-label="Close">${ic('x', 's18')}</button></div><div class="empty"><p>Loading the diff…</p></div>`, { label: 'Diff', cls: 'tall' });
  try {
    const r = await call(qualify(s.sessionId, 'git.diff'), { cwd: s.cwd, path, staged: false, untracked: true });
    const text = typeof r === 'string' ? r : (r && (r.diff || r.patch || r.text)) || '';
    const lines = text.split('\n').slice(0, 4000).map((l) => {
      const cls = l.startsWith('+') && !l.startsWith('+++') ? 'add' : l.startsWith('-') && !l.startsWith('---') ? 'del' : l.startsWith('@@') ? 'hunk' : '';
      return `<div class="${cls}">${esc(l) || ' '}</div>`;
    }).join('');
    sheet(`<div class="sh"><h3 class="mono small">${esc(path)}</h3><button class="ibtn" data-close aria-label="Close">${ic('x', 's18')}</button></div>
      <div class="diff scrolly">${lines || '<div class="muted">No differences.</div>'}</div>`, { label: 'Diff', cls: 'tall' });
  } catch (e) { closeSheet(); notice('Could not read the diff: ' + errText(e), 'error'); }
}

// ── Continue with… / Start fresh from a summary ─────────────────────────
const RANK = { default: 0, ask: 0, plan: 0, acceptEdits: 1, bypassPermissions: 2, yolo: 2 };

export async function openHandoff(s, { fresh, go }) {
  const source = providerOf(s) === 'codex' ? 'codex' : 'claude';
  const other = source === 'codex' ? 'claude' : 'codex';
  const st = { target: fresh ? source : other, brief: fresh || !isLive(s) ? 'summary' : 'agent', model: null, effort: '', mode: '', models: null, busy: false };
  const sourceMode = act.accessWire(source, permissionOf(s));
  const accessFor = (target) => {
    const modes = act.modesFor(target === 'claude' ? 'claude' : 'other');
    // Never wider than the source; full access also needs this token's grant.
    return modes.filter((m) => RANK[m.id] <= RANK[sourceMode] && (!act.fullAccess(m.id) || bus.scope.fullAccess));
  };
  const carry = (target) => {
    const ok = accessFor(target).map((m) => m.id);
    const wanted = act.accessWire(target, sourceMode);
    return ok.includes(wanted) ? wanted : ok[0];
  };
  st.mode = carry(st.target);
  const loadModels = async () => {
    st.models = null; draw();
    try { st.models = await act.listModels(st.target, s.cwd, s.sessionId); } catch { st.models = []; }
    if (st.target === source) {
      const cur = modelOf(s);
      st.model = st.models.find((m) => m.id === cur || m.legacy === cur) || null;
      st.effort = effortOf(s);
    }
    draw();
  };
  const sourceName = providerName(source);
  const html = () => {
    const tName = providerName(st.target);
    const title = st.target === source ? 'Start fresh from a summary' : `Continue with ${tName}`;
    const access = accessFor(st.target);
    const efforts = act.effortsFor(st.target);
    return `<div class="sh"><span class="ptile">${mark(st.target)}</span><h3>${esc(title)}</h3><button class="ibtn" data-close aria-label="Close">${ic('x', 's18')}</button></div>
    <p class="sheet-body">${st.target === source
      ? `A fresh ${esc(tName)} agent starts in this session’s folder with a summary of the work so far, so it does not re-read the whole conversation. This session stays as it is.`
      : `A new ${esc(tName)} session starts in this session’s folder with a brief of the work so far. This session stays as it is.`}</p>
    <div class="seg wide" role="tablist"><button class="${st.target === other ? 'on' : ''}" data-target="${other}">Continue with ${esc(providerName(other))}</button><button class="${st.target === source ? 'on' : ''}" data-target="${source}">Start a fresh ${esc(sourceName)}</button></div>
    <div class="glabel">This session</div>
    <div class="group"><div class="gi"><span class="tx muted">Agent</span><span class="v">${mark(source)}${esc(sourceName)} · ${esc(modelName(modelOf(s)) || 'default')}</span></div>
      <div class="gi"><span class="tx muted">Folder</span><span class="v mono small">${esc(s.cwd)}</span></div></div>
    <div class="glabel">${esc(tName)}</div>
    <div class="row2 ratio">
      <label class="select"><select data-model aria-label="Model">${st.models === null ? '<option>Loading…</option>' : `<option value="">Provider default</option>${st.models.map((m, i) => `<option value="${i}"${st.model === m ? ' selected' : ''}>${esc(m.label)}</option>`).join('')}`}</select>${ic('chevron-down', 's16 muted')}</label>
      <label class="select"><select data-effort aria-label="Effort"${efforts.length ? '' : ' disabled'}><option value="">Default</option>${efforts.map((e) => `<option${st.effort === e ? ' selected' : ''}>${e}</option>`).join('')}</select>${ic('chevron-down', 's16 muted')}</label>
    </div>
    <label class="select">${ic('shield-check', 's16 muted')}<select data-mode aria-label="Access">${access.map((m) => `<option value="${m.id}"${st.mode === m.id ? ' selected' : ''}>${esc(m.label)}</option>`).join('')}</select><span class="muted small">${st.mode === act.accessWire(st.target, sourceMode) ? 'same as this session' : 'narrower than this session'}</span></label>
    <div class="glabel">Brief</div>
    <div class="seg wide col" role="radiogroup">
      <button class="${st.brief === 'agent' ? 'on' : ''}" data-brief="agent"${isLive(s) ? '' : ' disabled'}>Written by ${esc(sourceName)}</button>
      <button class="${st.brief === 'summary' ? 'on' : ''}" data-brief="summary">Summary by a fast model</button>
      <button class="${st.brief === 'mechanical' ? 'on' : ''}" data-brief="mechanical">Quick summary</button>
    </div>
    <p class="muted small">${st.brief === 'agent' ? `${esc(sourceName)} stops its current work and writes the brief, which takes one turn and up to 2½ minutes. If it cannot, the hub uses a quick summary instead and says so.`
      : st.brief === 'summary' ? 'A fast model summarizes the conversation. Nothing is sent to this session, so a cold cache is not re-read.'
      : 'The hub writes a mechanical digest of the conversation right away.'}</p>
    <button class="btn block primary" data-go${st.busy ? ' disabled' : ''}>${st.busy ? (st.brief === 'agent' ? `Asking ${esc(sourceName)} to write the brief…` : st.brief === 'summary' ? 'A fast model is writing the summary…' : 'Summarizing the conversation…') : esc(title)} ${st.busy ? '' : ic('arrow-right', 's16')}</button>`;
  };
  let el = null;
  const draw = () => { if (el && el.isConnected) { el.innerHTML = '<div class="grab"></div>' + html(); bindAll(); } };
  const bindAll = () => {
    for (const b of el.querySelectorAll('[data-close]')) b.onclick = () => closeSheet();
    for (const b of el.querySelectorAll('[data-target]')) b.onclick = () => { st.target = b.dataset.target; st.mode = carry(st.target); st.model = null; st.effort = ''; loadModels(); };
    for (const b of el.querySelectorAll('[data-brief]')) b.onclick = () => { st.brief = b.dataset.brief; draw(); };
    const sel = el.querySelector('[data-model]');
    sel.onchange = () => { st.model = sel.value === '' ? null : st.models[Number(sel.value)]; };
    const ef = el.querySelector('[data-effort]');
    ef.onchange = () => { st.effort = ef.value; };
    const md = el.querySelector('[data-mode]');
    md.onchange = () => { st.mode = md.value; draw(); };
    el.querySelector('[data-go]').onclick = async () => {
      if (st.busy) return;
      st.busy = true; draw();
      const r = await act.handoff(s.sessionId, {
        provider: st.target, brief: st.brief, model: st.model && st.model.legacy, contextWindow: st.model && st.model.contextWindow,
        effort: st.effort, mode: st.mode,
      });
      st.busy = false;
      if (!r) { draw(); return; }
      drafts.set(r.sessionId, r.prompt);
      closeSheet();
      go('#/s/' + encodeURIComponent(r.sessionId));
    };
  };
  sheet(html(), { label: 'Handoff', cls: 'tall', bind(root) { el = root; bindAll(); } });
  loadModels();
}
