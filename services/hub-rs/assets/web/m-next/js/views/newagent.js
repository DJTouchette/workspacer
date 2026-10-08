// New agent (native ui/launch.rs order): Project card → Agent → Model +
// Refresh / Effort → Task → Options (access, session name, continue a
// conversation) → a sticky footer summary with Start agent.
import { call, can, bus } from '../bus.js';
import { cfg, recents, seed, sessions, drafts } from '../store.js';
import { esc, ic, mark, modelName, errText, providerName, basename, when } from '../util.js';
import { notice, pick, sheet, closeSheet } from '../ui.js';
import * as act from '../actions.js';
import { projectList, inspect, absoluteDir } from '../projects.js';

const AGENTS = [
  { id: 'claude', name: 'Claude', sub: 'Claude Code' },
  { id: 'codex', name: 'Codex', sub: 'OpenAI Codex' },
];
let probe = null;   // providers.checkAll, once per app run

export function mount(root, ctx, params) {
  const st = {
    cwd: params.get('cwd') || '',
    provider: (cfg && cfg.agents && cfg.agents.defaultProvider === 'codex') ? 'codex' : 'claude',
    model: null, models: null, modelsError: '', effort: '', mode: '', label: '', resume: null,
    task: '', inspection: null, busy: false,
  };
  st.mode = st.provider === 'claude' ? 'default' : 'ask';
  root.innerHTML = `<div class="screen page-screen">
    <div class="pagehead"><span class="grow"></span><button class="ibtn" data-x aria-label="Close">${ic('x', 's20')}</button></div>
    <div class="page scrolly" data-page></div>
    <div class="footerbar" data-footer></div>
  </div>`;
  const $ = (s) => root.querySelector(s);
  const page = $('[data-page]');
  $('[data-x]').onclick = () => ctx.back();

  if (!st.cwd) {
    const list = projectList();
    st.cwd = (list[0] && list[0].path) || '';
  }

  async function loadModels(force) {
    st.models = null; st.modelsError = ''; render();
    if (!can('agents.spawn')) { st.models = []; render(); return; }
    try { st.models = await act.listModels(st.provider, st.cwd); }
    catch (e) { st.models = []; st.modelsError = 'Could not load models: ' + errText(e); }
    if (st.model && !st.models.some((m) => m.id === st.model.id && m.contextWindow === st.model.contextWindow)) st.model = null;
    render();
    void force;
  }
  async function loadInspection() {
    st.inspection = null;
    if (!st.cwd || !can('fs.listDir')) { render(); return; }
    const cwd = st.cwd;
    const r = await inspect(cwd);
    if (cwd === st.cwd) { st.inspection = r; render(); }
  }

  function projectCard() {
    const list = projectList();
    const p = list.find((x) => x.path === st.cwd) || (st.cwd ? { path: st.cwd, name: basename(st.cwd), tag: basename(st.cwd).slice(0, 2).toUpperCase(), tint: '#6b8afd', running: 0 } : null);
    if (!p) {
      return `<div class="group pad"><div class="field"><div class="fl">Project</div>
        <button class="btn block" data-change>${ic('folder-open', 's16')}Choose a project folder</button></div></div>`;
    }
    const i = st.inspection;
    const git = !i ? '<span class="muted">Checking the folder…</span>'
      : !i.exists ? `<span class="t-error">${ic('triangle-alert', 's12')}Folder not found</span>`
      : i.branch ? `<span class="t-success">${ic('git-branch', 's12')}${esc(i.branch)}${i.changes ? ` · ${i.changes} uncommitted change${i.changes === 1 ? '' : 's'}` : ' · clean'}</span>`
      : `<span class="muted">${i.error ? esc(i.error) : 'Not a git repository'}</span>`;
    return `<div class="group pad"><div class="field"><div class="fl">Project</div>
      <div class="proj"><span class="ptag" style="color:${p.tint};background:color-mix(in srgb, ${p.tint} 22%, transparent)">${esc(p.tag)}</span>
        <div class="grow minw"><div class="pn">${esc(p.name)}${p.running ? ` <span class="badge neutral">${p.running} running</span>` : ''}</div>
        <div class="mono muted small ell">${esc(p.path)}</div>
        <div class="small gitline">${can('fs.listDir') ? git : ''}</div></div>
        <button class="btn sm" data-change>Change</button></div></div></div>`;
  }

  function render() {
    const efforts = act.effortsFor(st.provider);
    const modes = act.modesFor(st.provider === 'claude' ? 'claude' : 'other');
    const installed = (p) => !probe || !probe.find((x) => x.provider === p) || probe.find((x) => x.provider === p).found;
    const allowed = can('agents.spawn');
    page.innerHTML = `
      <div class="ptitle"><span class="overline">New agent</span><h1>Start an agent</h1></div>
      ${projectCard()}
      <div class="field"><div class="fl">Agent</div>
        <div class="agentpick">${AGENTS.map((a) => `<button class="${st.provider === a.id ? 'on' : ''}" data-agent="${a.id}"${installed(a.id) ? '' : ' disabled'} aria-pressed="${st.provider === a.id}">
          <span class="ptile">${mark(a.id)}</span><span class="grow"><span class="an">${esc(a.name)}</span><span class="as">${installed(a.id) ? esc(a.sub) : 'Not installed'}</span></span>${st.provider === a.id ? ic('check', 's16 t-accent') : ''}</button>`).join('')}</div></div>
      <div class="row2 ratio">
        <div class="field"><div class="fl">Model <button class="link small" data-refresh>${ic('redo', 's12')}Refresh</button></div>
          <label class="select"><select data-model aria-label="Model"${st.models === null ? ' disabled' : ''}>
            <option value="">${st.models === null ? 'Loading…' : 'Provider default'}</option>
            ${(st.models || []).map((m, i) => `<option value="${i}"${st.model && st.model.id === m.id && st.model.contextWindow === m.contextWindow ? ' selected' : ''}>${esc(m.label)}</option>`).join('')}
          </select>${ic('chevron-down', 's16 muted')}</label></div>
        <div class="field"><div class="fl">Effort</div>
          <label class="select"><select data-effort aria-label="Effort"${efforts.length ? '' : ' disabled'}><option value="">Default</option>
            ${efforts.map((e) => `<option value="${e}"${st.effort === e ? ' selected' : ''}>${e.charAt(0).toUpperCase() + e.slice(1)}</option>`).join('')}</select>${ic('chevron-down', 's16 muted')}</label></div>
      </div>
      ${st.modelsError ? `<div class="t-error small">${esc(st.modelsError)}</div>` : ''}
      <div class="field"><div class="fl">Task <span class="muted normal">Optional · you can also start empty</span></div>
        <textarea class="taskbox" data-task rows="4" placeholder="What would you like to work on?" aria-label="Task"></textarea></div>
      <div class="field"><div class="fl">Options</div>
        <div class="chips">
          <button class="chip on" data-access>${ic('shield-check', 's14')}${esc(act.accessLabel(st.mode))}</button>
          <button class="chip${st.label ? ' on' : ''}" data-name>${ic('type', 's14')}${st.label ? esc(st.label) : 'Session name'}</button>
          <button class="chip${st.resume ? ' on' : ''}" data-resume>${ic('history', 's14')}${st.resume ? esc(st.resume.title) : 'Continue a conversation'}</button>
        </div>
        ${!allowed ? `<div class="muted small">This device holds a ${esc(bus.scope.name || 'scoped')} token — starting an agent needs operator access.</div>` : ''}
      </div>`;
    const ta = $('[data-task]');
    ta.value = st.task;
    ta.oninput = () => { st.task = ta.value; };
    $('[data-model]').onchange = (e) => { st.model = e.target.value === '' ? null : st.models[Number(e.target.value)]; renderFooter(); };
    $('[data-effort]').onchange = (e) => { st.effort = e.target.value; renderFooter(); };
    renderFooter();
    void modes;
  }
  function renderFooter() {
    const ok = can('agents.spawn') && absoluteDir(st.cwd) && !st.busy && !(st.inspection && st.inspection.exists === false);
    $('[data-footer]').innerHTML = `<span class="sum">${esc(providerName(st.provider))} · ${esc(st.model ? st.model.label : 'Provider default')}${st.effort ? ' · ' + esc(st.effort) : ''}<br>${esc(basename(st.cwd) || 'No project')}</span>
      <button class="btn primary big" data-start${ok ? '' : ' disabled'}>${st.busy ? 'Starting…' : st.resume ? 'Resume' : 'Start agent'} ${ic('arrow-right', 's16')}</button>`;
    $('[data-start]').onclick = start;
  }

  page.addEventListener('click', async (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    if (b.dataset.agent) {
      st.provider = b.dataset.agent; st.mode = st.provider === 'claude' ? 'default' : 'ask'; st.effort = ''; st.model = null;
      if (st.resume && st.resume.provider !== st.provider) st.resume = null;
      loadModels();
    } else if (b.hasAttribute('data-refresh')) loadModels(true);
    else if (b.hasAttribute('data-change')) chooseProject();
    else if (b.hasAttribute('data-access')) {
      const v = await pick('Access', act.modesFor(st.provider === 'claude' ? 'claude' : 'other').map((m) => ({
        value: m.id, label: m.label, detail: act.fullAccess(m.id) && !bus.scope.fullAccess ? 'Needs an operator pairing' : m.detail,
        on: st.mode === m.id, disabled: act.fullAccess(m.id) && !bus.scope.fullAccess,
      })));
      if (v) { st.mode = v; render(); }
    } else if (b.hasAttribute('data-name')) nameSheet();
    else if (b.hasAttribute('data-resume')) chooseResume();
  });

  function nameSheet() {
    sheet(`<div class="sh"><h3>Session name</h3></div>
      <input class="input" data-in maxlength="80" placeholder="Name this session" value="${esc(st.label)}" aria-label="Session name">
      <p class="muted small">Without a name, the hub names it after its first exchange.</p>
      <div class="row2"><button class="btn block" data-clear>Clear</button><button class="btn block primary" data-ok>Done</button></div>`, {
      label: 'Session name',
      bind(el, close) {
        const inp = el.querySelector('[data-in]');
        setTimeout(() => inp.focus(), 50);
        el.querySelector('[data-ok]').onclick = () => { st.label = inp.value.trim(); close(); render(); };
        el.querySelector('[data-clear]').onclick = () => { st.label = ''; close(); render(); };
        inp.onkeydown = (ev) => { if (ev.key === 'Enter') el.querySelector('[data-ok]').click(); };
      },
    });
  }
  async function chooseResume() {
    const rows = (recents || []).filter((r) => r && r.sessionId && r.cwd && (r.provider || 'claude') === st.provider && !sessions.has(r.sessionId)).slice(0, 30);
    const v = await pick('Continue a conversation', [
      { value: '', label: 'Start a new conversation', on: !st.resume },
      ...rows.map((r) => ({ value: r.sessionId, label: r.title || r.name || basename(r.cwd), detail: `${basename(r.cwd)} · ${when(r.updatedAt || r.startedAt)}`, on: st.resume && st.resume.id === r.sessionId })),
    ]);
    if (v === undefined) return;
    const r = rows.find((x) => x.sessionId === v);
    st.resume = r ? { id: r.sessionId, title: r.title || r.name || basename(r.cwd), provider: r.provider || 'claude' } : null;
    if (r) { st.cwd = r.cwd; loadInspection(); }
    render();
  }
  async function chooseProject() {
    const list = projectList();
    const v = await pick('Project', [
      ...list.map((p) => ({ value: p.path, label: p.name + (p.pinned ? ' ★' : ''), detail: p.path, on: p.path === st.cwd })),
      ...(can('fs.listDir') ? [{ value: '\u0000browse', label: 'Browse the hub’s folders…' }] : []),
    ]);
    if (v === undefined) return;
    if (v === '\u0000browse') { browse(st.cwd || ''); return; }
    st.cwd = v; loadInspection(); loadModels(); render();
  }
  async function browse(path) {
    let r;
    try { r = await call('fs.listDir', { path }); } catch (e) { notice(errText(e), 'error'); return; }
    if (!r || typeof r.path !== 'string') return;
    const join = (p, n) => p.replace(/[\\/]$/, '') + (p.includes('\\') && !p.includes('/') ? '\\' : '/') + n;
    const v = await pick(r.path, [
      { value: '\u0000use', label: 'Use this folder', detail: r.path },
      ...(r.parent && r.parent !== r.path ? [{ value: '\u0000up', label: 'Up one level' }] : []),
      ...(r.dirs || []).slice(0, 300).map((d) => ({ value: d, label: d + '/' })),
    ]);
    if (v === undefined) return;
    if (v === '\u0000use') { st.cwd = r.path; loadInspection(); loadModels(); render(); }
    else if (v === '\u0000up') browse(r.parent);
    else browse(join(r.path, v));
  }

  async function start() {
    if (st.busy) return;
    const wire = act.accessWire(st.provider, st.mode);
    if (act.fullAccess(wire) && !bus.scope.fullAccess) { notice('Full access needs an operator pairing', 'warning'); return; }
    const params = { provider: st.provider, cwd: st.cwd.trim(), transport: 'stream', permissionMode: wire, skipPermissions: act.fullAccess(wire) };
    if (st.model) { params.model = st.model.legacy; if (st.model.contextWindow != null) params.contextWindow = st.model.contextWindow; }
    if (st.effort) params.effort = st.effort;
    if (st.label) params.label = st.label; else params.autoTitle = true;
    const task = st.task.trim();
    if (task) params.message = task;
    if (st.resume) params.resumeSessionId = st.resume.id;
    st.busy = true; renderFooter();
    try {
      const r = await call('agents.spawn', params, 60000);
      const id = r && r.sessionId;
      // An older host answers without messageQueued: send the task the old way.
      if (id && task && !(r && r.messageQueued)) {
        try { await call('agents.sendMessage', { sessionId: id, text: task }); }
        catch (e) { drafts.set(id, task); notice('Started, but the task was not sent: ' + errText(e), 'warning'); }
      }
      await seed();
      notice('Agent started', 'success');
      if (id) ctx.go('#/s/' + encodeURIComponent(id), true); else ctx.go('#/', true);
    } catch (e) {
      notice('Could not start the agent: ' + errText(e), 'error');
    } finally { st.busy = false; if (root.isConnected) renderFooter(); }
  }

  if (!probe && can('providers.checkAll')) {
    call('providers.checkAll', {}).then((r) => { probe = Array.isArray(r) ? r : null; render(); }).catch(() => {});
  }
  render();
  loadModels();
  loadInspection();
  return { update() { /* the form owns its state; project counts refresh on next open */ }, destroy() { closeSheet(); } };
}
