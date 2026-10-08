// Remote worker nodes (machines that can be off on purpose): seeded from
// nodes.list, patched from node.state_changed, never polled. Ported from /m;
// a node that is quietly fine says nothing, except a connected machine this
// phone may switch off (it is billing, so the off switch lives here).
import { call, can } from '../bus.js';
import { nodes, nodeState, applyNodeState, changed } from '../store.js';
import { esc, spinner } from '../util.js';
import { ask } from '../ui.js';

const META = {
  available: { label: 'Connected', tone: 'success', d: 'This machine is on the bus and answering.' },
  waking: { label: 'Starting', tone: 'busy', spin: true, d: 'The machine is booting — usually ready in about 20 seconds.' },
  stopping: { label: 'Shutting down', tone: 'busy', spin: true, d: 'The machine is shutting down cleanly. It stops billing once it is off.' },
  stopped: { label: 'Asleep', tone: 'muted', d: 'Switched off, and nothing is billing. Connecting will start it.' },
  unreachable: { label: "Can't reach", tone: 'warning', d: "The hub can't get a working machine out of this one." },
};
const COST = 'Starts a real machine. It bills from boot until you put it back to sleep.';
const SLEEP = 'Shuts the machine down. Anything still running on it stops, and it stops billing.';

function crash(n) {
  const e = n.lastExit;
  if (!e || !e.reason || String(e.reason).indexOf('signal-') === 0) return '';
  return `Its previous run did not end cleanly: ${e.reason}${e.exitCode !== undefined ? ` (exit ${e.exitCode})` : ''}${e.at ? ` at ${e.at}` : ''}.`;
}
const mayBeRunning = (n) => n.state === 'available' || n.mayBeRunning === true;

export function nodesHtml() {
  if (!nodes || !nodes.length) return '';
  const mayWake = can('nodes.wake'), maySleep = can('nodes.sleep');
  const shown = nodes.filter((n) => n.state !== 'available' || crash(n) || (maySleep && n.wakeable));
  if (!shown.length) return '';
  return shown.map((n) => {
    const m = META[n.state] || META.unreachable;
    const pending = nodeState.pending.has(n.id), sleeping = nodeState.sleeping.has(n.id);
    const err = nodeState.errors.get(n.id) || '';
    let btn = '', note = '';
    if (n.state === 'stopping' || sleeping) btn = '<button class="btn sm" disabled>Shutting down…</button>';
    else if (n.state === 'waking' || pending) btn = '<button class="btn sm" disabled>Starting…</button>';
    else if (mayBeRunning(n)) {
      if (!n.wakeable) { btn = '<button class="btn sm" disabled>Put to sleep</button>'; note = 'This hub holds no cloud credentials for this machine.'; }
      else if (!maySleep) { btn = '<button class="btn sm" disabled>Put to sleep</button>'; note = 'Stopping a machine ends the work running on it — it needs an operator token.'; }
      else { btn = `<button class="btn sm" data-sleep="${esc(n.id)}">Put to sleep</button>`; note = SLEEP; }
    } else if (n.state !== 'available') {
      if (!n.wakeable) { btn = '<button class="btn sm" disabled>Connect</button>'; note = 'This hub holds no cloud credentials for this machine.'; }
      else if (!mayWake) { btn = '<button class="btn sm" disabled>Connect</button>'; note = 'Starting a machine spends money — it needs an operator token.'; }
      else { btn = `<button class="btn sm primary" data-wake="${esc(n.id)}">Connect</button>`; note = COST; }
    }
    const fails = n.wakeFailures > 0 ? `${n.wakeFailures} wake${n.wakeFailures === 1 ? '' : 's'} failed. The machine started and never became usable — check its boot log.` : '';
    return `<div class="card node" data-node="${esc(n.id)}" data-node-state="${esc(n.state)}">
      <div class="nh">${m.spin ? spinner() : `<span class="dot t-${m.tone}"></span>`}<b>${esc(n.label || n.id)}</b><span class="st t-${m.tone}">${esc(m.label)}</span></div>
      <div class="nd">${esc(n.detail || m.d)}</div>
      ${crash(n) ? `<div class="nd t-warning">${esc(crash(n))}</div>` : ''}${fails ? `<div class="nd t-warning">${esc(fails)}</div>` : ''}
      ${err ? `<div class="nd t-error">${esc(err)}</div>` : ''}
      ${btn ? `<div class="na">${btn}${note ? `<span class="faint">${esc(note)}</span>` : ''}</div>` : ''}
    </div>`;
  }).join('');
}

const refusal = (msg, verb) => /requires host authority/.test(msg) ? `${verb} a machine needs an operator token.`
  : /unknown node|naming a registered node is required/.test(msg) ? 'This machine is no longer in the registry.'
  : /cannot be put to sleep from here|has no cloud coordinates or credential/.test(msg) ? 'This hub holds no cloud credentials for this machine.'
  : msg || `Couldn't ${verb.toLowerCase()} the machine.`;

export function bindNodes(root) {
  root.addEventListener('click', async (e) => {
    const w = e.target.closest('[data-wake]'), s = e.target.closest('[data-sleep]');
    if (w) {
      const id = w.dataset.wake;
      if (!(await ask({ title: 'Connect this machine?', body: COST, confirm: 'Connect' }))) return;
      nodeState.pending.add(id); nodeState.errors.delete(id); changed();
      try {
        const n = await call('nodes.wake', { id });
        if (n && n.id) applyNodeState(n); else { nodeState.pending.delete(id); changed(); }
      } catch (err) { nodeState.pending.delete(id); nodeState.errors.set(id, refusal(String(err && err.message || ''), 'Starting')); changed(); }
    } else if (s) {
      const id = s.dataset.sleep;
      if (!(await ask({ title: 'Put this machine to sleep?', body: SLEEP, confirm: 'Put to sleep', danger: true }))) return;
      nodeState.sleeping.add(id); nodeState.errors.delete(id); changed();
      try {
        // Only an id goes out: the signal and drain window are the hub's.
        const n = await call('nodes.sleep', { id });
        if (n && n.id) applyNodeState(n); else { nodeState.sleeping.delete(id); changed(); }
      } catch (err) { nodeState.sleeping.delete(id); nodeState.errors.set(id, refusal(String(err && err.message || ''), 'Stopping')); changed(); }
    }
  });
}


