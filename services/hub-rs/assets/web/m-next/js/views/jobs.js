// Jobs (native jobs.rs / ui/jobs.rs). Agents write jobs; the phone does the
// owner's few writes: approve or reject a proposal, pause/resume, run now.
// Every write is a whole-spec jobs.upsert built from the listed row (live
// fields stripped), so fields this client does not know survive it.
import { call, can } from '../bus.js';
import { jobs, loadJobs } from '../store.js';
import { esc, ic, when, errText } from '../util.js';
import { notice, ask } from '../ui.js';

const LIVE = ['nextRunAt', 'running', 'lastRun'];
const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const spec = (job) => { const s = { ...job }; for (const k of LIVE) delete s[k]; return s; };
const proposed = (j) => !!String(j.proposedBy || '').trim();

function trigger(t) {
  t = t || {};
  if (t.kind === 'interval') { const m = Number(t.everyMinutes) || 0; return m > 0 && m % 60 === 0 ? `every ${m / 60}h` : `every ${m}m`; }
  if (t.kind === 'daily') { const days = (t.days || []).map((d) => DAYS[d]).filter(Boolean); return `daily ${t.at || '?'}${days.length ? ' · ' + days.join(' ') : ''}`; }
  if (t.kind === 'once') return t.once ? `once, ${t.once}` : 'once';
  if (t.kind === 'manual') return 'manual';
  return t.kind || '';
}
function action(a) {
  a = a || {};
  if (a.kind === 'spawn') { const n = (a.spawn && a.spawn.context || []).length; return `${n ? `${n} step${n === 1 ? '' : 's'} → ` : ''}agent in ${(a.spawn && a.spawn.cwd) || '?'}`; }
  if (a.kind === 'call') return `call ${(a.call && a.call.method) || '?'}`;
  if (a.kind === 'shell') return `$ ${(a.shell && a.shell.command) || '?'}`;
  return a.kind || '';
}
/** Everything the job will do: read before approving (there is no editor). */
function details(j) {
  const rows = [['Name', j.name || j.id], ['When', trigger(j.trigger)]];
  const a = j.action || {};
  if (a.kind === 'spawn') {
    const s = a.spawn || {};
    rows.push(['Agent in', s.cwd || '']);
    const using = ['provider', 'model', 'effort', 'permissionMode'].map((k) => s[k]).filter(Boolean);
    if (using.length) rows.push(['Using', using.join(' · ')]);
    (s.context || []).forEach((step, i) => rows.push([`Step ${i + 1}`, typeof step === 'string' ? step : JSON.stringify(step)]));
    rows.push(['Prompt', s.prompt || '']);
  } else if (a.kind === 'shell') {
    rows.push(['Runs', `$ ${(a.shell && a.shell.command) || ''}`]);
    if (a.shell && a.shell.cwd) rows.push(['In', a.shell.cwd]);
  } else if (a.kind === 'call') {
    rows.push(['Calls', (a.call && a.call.method) || '']);
    if (a.call && a.call.params != null) rows.push(['Params', JSON.stringify(a.call.params, null, 2)]);
  }
  return rows;
}

export function mount(root, ctx) {
  root.innerHTML = `<div class="screen page-screen">
    <div class="pagehead"><button class="ibtn" data-back aria-label="Back">${ic('chevron-left', 's20')}</button><span class="grow"></span>
      <button class="ibtn" data-refresh aria-label="Refresh">${ic('refresh-cw', 's18')}</button></div>
    <div class="page scrolly">
      <div class="ptitle"><span class="overline">Workspace</span><h1>Jobs</h1><p>Work the hub runs on a schedule. Agents propose jobs; nothing runs until you approve it.</p></div>
      <div data-list></div>
    </div>
  </div>`;
  const $ = (s) => root.querySelector(s);
  $('[data-back]').onclick = () => ctx.back();
  $('[data-refresh]').onclick = () => loadJobs();
  const open = new Set();
  const write = can('jobs.upsert');
  $('[data-list]').onclick = async (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    const j = (jobs || []).find((x) => x.id === b.dataset.id);
    if (!j) return;
    const run = async (label, fn) => {
      b.disabled = true;
      try { await fn(); notice(label, 'success'); } catch (err) { notice(errText(err), 'error'); }
      await loadJobs();
    };
    if (b.hasAttribute('data-toggle-job')) { open.has(j.id) ? open.delete(j.id) : open.add(j.id); update(); }
    else if (b.hasAttribute('data-approve')) {
      // Approving clears proposedBy and arms it; a `replaces` proposal is applied in place by the hub.
      const s = spec(j); s.enabled = true; delete s.proposedBy;
      run(j.replaces ? 'Change approved' : 'Job approved', () => call('jobs.upsert', s));
    } else if (b.hasAttribute('data-reject')) {
      if (await ask({ title: 'Reject this proposal?', body: `“${j.name || j.id}” is removed and will not run.`, confirm: 'Reject', danger: true })) run('Proposal rejected', () => call('jobs.remove', { id: j.id }));
    } else if (b.hasAttribute('data-pause')) {
      const s = spec(j); s.enabled = !j.enabled;
      run(j.enabled ? 'Paused' : 'Resumed', () => call('jobs.upsert', s));
    } else if (b.hasAttribute('data-run')) {
      run('Started', async () => { const r = await call('jobs.run', { id: j.id }); if (r && r.started !== true) throw new Error('The job did not run: ' + ((r && r.reason) || 'it did not start')); });
    }
  };
  function update() {
    if (!can('jobs.list')) { $('[data-list]').innerHTML = `<div class="empty"><b>Jobs need an operator token</b></div>`; return; }
    if (jobs === null) { $('[data-list]').innerHTML = `<div class="empty"><p>Loading jobs…</p></div>`; return; }
    const list = [...jobs].sort((a, b) => Number(proposed(b)) - Number(proposed(a)));
    if (!list.length) { $('[data-list]').innerHTML = `<div class="empty">${ic('calendar', 's20 muted')}<b>No jobs</b><p>Ask an agent to schedule recurring work.</p></div>`; return; }
    $('[data-list]').innerHTML = list.map((j) => {
      const isProp = proposed(j);
      const last = j.lastRun && j.lastRun.startedAt ? `${j.lastRun.status === 'ok' ? 'ok' : j.lastRun.status === 'skipped' ? 'skipped' : 'failed'} ${when(j.lastRun.startedAt)}` : '';
      const st = isProp ? '<span class="badge warning">Proposed</span>' : j.running ? '<span class="badge busy">Running</span>' : j.enabled ? '<span class="badge success">On</span>' : '<span class="badge neutral">Paused</span>';
      const isOpen = open.has(j.id) || isProp;
      return `<div class="card job${isProp ? ' prop' : ''}" data-job="${esc(j.id)}">
        <button class="jh" data-toggle-job data-id="${esc(j.id)}">${ic('calendar', 's16 t-accent')}<span class="tx"><span class="n">${esc(j.name || j.id)}</span>
          <span class="m">${esc(trigger(j.trigger))} · ${esc(action(j.action))}</span>
          ${isProp ? `<span class="m t-warning">${j.replaces ? `Proposes a change to ${esc(j.replaces)} · ` : ''}from ${esc(j.proposedBy)}</span>` : `<span class="m">${j.nextRunAt ? `Next ${esc(when(j.nextRunAt))}` : ''}${last ? ` · last ${esc(last)}` : ''}</span>`}</span>${st}</button>
        ${isOpen ? `<div class="jd">${details(j).map(([k, v]) => `<div class="kv"><span class="muted">${esc(k)}</span><span class="${k === 'Params' || k === 'Runs' ? 'mono' : ''}">${esc(v)}</span></div>`).join('')}</div>` : ''}
        ${write ? `<div class="ja">${isProp
          ? `<button class="btn sm" data-reject data-id="${esc(j.id)}">Reject</button><button class="btn sm primary" data-approve data-id="${esc(j.id)}">${j.replaces ? 'Approve change' : 'Approve'}</button>`
          : `<button class="btn sm" data-pause data-id="${esc(j.id)}">${j.enabled ? 'Pause' : 'Resume'}</button><button class="btn sm" data-run data-id="${esc(j.id)}"${j.running ? ' disabled' : ''}>${ic('play', 's14')}Run now</button>`}</div>` : ''}
      </div>`;
    }).join('');
  }
  loadJobs();
  update();
  return { update };
}
