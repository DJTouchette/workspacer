#!/usr/bin/env node
/**
 * Jobs harness — end-to-end exercise of the hub's job system against a REAL
 * scratch hub, with a fake agents.spawn provider on the bus (the desktop's
 * role) so spawn-action jobs complete without any claudemon.
 *
 *   node services/hub/scripts/jobs-harness.mjs
 *
 * Boots a hub on 127.0.0.1:18897 with an isolated temp state dir (never the
 * live desktop hub's 7895; WORKSPACER_PARENT_PID is stripped so a hub started
 * from inside a workspacer session doesn't self-exit). Then, over the real
 * bus protocol:
 *
 *   - validation refusals (nameless job, call action targeting jobs.* /
 *     hub:<peer>/);
 *   - a manual spawn job: run-now → the provider sees agents.spawn (label,
 *     cwd) then agents.sendMessage (prompt) → history records ok;
 *   - a shell job (real /bin/sh) → output tail lands in the run detail;
 *   - a failing shell job → error run + a notify.post event on the bus;
 *   - overlap: run-now on a still-running job answers started:false;
 *   - a daily job lists a future nextRunAt;
 *   - persistence: the hub is killed and restarted on the same state dir and
 *     the jobs (and run history) come back.
 *
 * Exit code 0 = every check passed.
 */
import { spawn, execSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const hubDir = path.join(here, '..');
const PORT = 18897;
const TOKEN = 'jobs-harness-token';
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-jobs-harness-'));
const hubBin = path.join(tmp, 'hub');

let failures = 0;
const check = (name, cond, extra = '') => {
  const ok = !!cond;
  console.log(`${ok ? '  ✓' : '  ✗'} ${name}${!ok && extra ? ` — ${extra}` : ''}`);
  if (!ok) failures++;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

console.log('building hub…');
execSync(`go build -o ${hubBin} ./cmd/hub`, { cwd: hubDir, stdio: 'inherit' });

let hub = null;
function startHub() {
  const env = { ...process.env };
  delete env.WORKSPACER_PARENT_PID; // parentwatch would kill a nested hub
  hub = spawn(
    hubBin,
    [
      '-addr', `127.0.0.1:${PORT}`,
      '-token', TOKEN,
      '-jobs-file', path.join(tmp, 'jobs.json'),
      '-layout-file', path.join(tmp, 'layout.json'),
      '-push-dir', path.join(tmp, 'push'),
      '-peers-file', path.join(tmp, 'peers.json'),
      '-tokens-file', '',
    ],
    { env, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  hub.stderr.on('data', (d) => process.env.HARNESS_VERBOSE && process.stderr.write(d));
  hub.stdout.on('data', (d) => process.env.HARNESS_VERBOSE && process.stdout.write(d));
}

/** Tiny bus client over node's native WebSocket. */
function connect(name) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${PORT}/bus?token=${TOKEN}`);
    let seq = 0;
    const pending = new Map();
    const providers = new Map();
    const events = [];
    ws.onopen = () =>
      resolve({
        ws,
        events,
        provide(method, fn) {
          providers.set(method, fn);
        },
        register(methods) {
          ws.send(JSON.stringify({ op: 'register', methods }));
        },
        subscribe(...topics) {
          ws.send(JSON.stringify({ op: 'subscribe', topics }));
        },
        call(method, params = {}) {
          return new Promise((res, rej) => {
            const id = `${name}-${++seq}`;
            pending.set(id, { res, rej });
            setTimeout(() => {
              if (pending.delete(id)) rej(new Error(`timeout: ${method}`));
            }, 20000);
            ws.send(JSON.stringify({ op: 'call', id, method, params }));
          });
        },
        close() {
          try {
            ws.close();
          } catch {}
        },
      });
    ws.onerror = (e) => reject(new Error(`ws ${name}: ${e.message ?? 'error'}`));
    ws.onmessage = async (m) => {
      const f = JSON.parse(m.data.toString());
      if (f.op === 'event') events.push(f.event);
      else if (f.op === 'call') {
        const fn = providers.get(f.method);
        if (!fn) {
          ws.send(JSON.stringify({ op: 'error', id: f.id, error: `no handler ${f.method}` }));
          return;
        }
        try {
          const result = await fn(f.params ?? {});
          ws.send(JSON.stringify({ op: 'result', id: f.id, result: result ?? null }));
        } catch (err) {
          ws.send(JSON.stringify({ op: 'error', id: f.id, error: String(err?.message ?? err) }));
        }
      } else if (f.op === 'result' || f.op === 'error') {
        const p = pending.get(f.id);
        if (!p) return;
        pending.delete(f.id);
        f.op === 'result' ? p.res(f.result) : p.rej(new Error(f.error));
      }
    };
  });
}

async function connectWithRetry(name, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try {
      return await connect(name);
    } catch {
      await sleep(250);
    }
  }
  throw new Error('hub never came up');
}

async function waitFor(fn, ms = 8000) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() > end) return null;
    await sleep(150);
  }
}

const lastRunOf = async (caller, id) => {
  const res = await caller.call('jobs.history', { id });
  return res?.runs?.[0] ?? null;
};

try {
  startHub();

  // The provider — plays the desktop main's role on the bus.
  const provider = await connectWithRetry('prov');
  const spawnCalls = [];
  const msgCalls = [];
  provider.provide('agents.spawn', (p) => {
    spawnCalls.push(p);
    return { sessionId: 'harness-sess-1' };
  });
  provider.provide('agents.sendMessage', (p) => {
    msgCalls.push(p);
    return { ok: true };
  });
  provider.register(['agents.spawn', 'agents.sendMessage']);
  provider.subscribe('*');

  const caller = await connectWithRetry('call');
  await sleep(300); // let registrations settle

  console.log('\nvalidation:');
  check(
    'nameless job refused',
    await caller
      .call('jobs.upsert', { name: ' ', enabled: true, trigger: { kind: 'manual' }, action: { kind: 'shell', shell: { command: 'true' } } })
      .then(() => false)
      .catch((e) => /name/.test(e.message)),
  );
  check(
    'call action into jobs.* refused (recursion)',
    await caller
      .call('jobs.upsert', { name: 'evil', enabled: true, trigger: { kind: 'manual' }, action: { kind: 'call', call: { method: 'jobs.run' } } })
      .then(() => false)
      .catch((e) => /may not target/.test(e.message)),
  );
  check(
    'call action across federation refused',
    await caller
      .call('jobs.upsert', { name: 'evil2', enabled: true, trigger: { kind: 'manual' }, action: { kind: 'call', call: { method: 'hub:work/agents.spawn' } } })
      .then(() => false)
      .catch((e) => /may not target/.test(e.message)),
  );

  console.log('\nspawn job:');
  const spawnJob = await caller.call('jobs.upsert', {
    name: 'Harness triage',
    enabled: true,
    trigger: { kind: 'manual' },
    action: {
      kind: 'spawn',
      spawn: { cwd: '/tmp/harness-repo', prompt: 'triage the overnight queue', provider: 'claude' },
    },
  });
  check('upsert minted an id', typeof spawnJob.id === 'string' && spawnJob.id.length > 0);
  const runRes = await caller.call('jobs.run', { id: spawnJob.id });
  check('run-now started', runRes?.started === true);
  const spawnRun = await waitFor(async () => {
    const r = await lastRunOf(caller, spawnJob.id);
    return r && r.status !== undefined ? r : null;
  });
  check('run recorded ok', spawnRun?.status === 'ok', JSON.stringify(spawnRun));
  check('detail names the session', /harness-sess-1/.test(spawnRun?.detail ?? ''));
  check('provider saw agents.spawn with cwd + label', spawnCalls.length === 1 && spawnCalls[0].cwd === '/tmp/harness-repo' && spawnCalls[0].label === 'Harness triage', JSON.stringify(spawnCalls));
  check('prompt followed via agents.sendMessage', msgCalls.length === 1 && msgCalls[0].sessionId === 'harness-sess-1' && msgCalls[0].text === 'triage the overnight queue', JSON.stringify(msgCalls));

  console.log('\nshell jobs:');
  const shellJob = await caller.call('jobs.upsert', {
    name: 'Harness echo',
    enabled: true,
    trigger: { kind: 'manual' },
    action: { kind: 'shell', shell: { command: 'echo hello-from-the-harness' } },
  });
  await caller.call('jobs.run', { id: shellJob.id });
  const shellRun = await waitFor(() => lastRunOf(caller, shellJob.id));
  check('shell run ok with output tail', shellRun?.status === 'ok' && /hello-from-the-harness/.test(shellRun?.detail ?? ''), JSON.stringify(shellRun));

  const failJob = await caller.call('jobs.upsert', {
    name: 'Harness failure',
    enabled: true,
    trigger: { kind: 'manual' },
    action: { kind: 'shell', shell: { command: 'echo boom-detail >&2; exit 3' } },
  });
  await caller.call('jobs.run', { id: failJob.id });
  const failRun = await waitFor(async () => {
    const r = await lastRunOf(caller, failJob.id);
    return r?.status === 'error' ? r : null;
  });
  check('failing shell records an error run', failRun?.status === 'error' && /boom-detail/.test(failRun?.detail ?? ''), JSON.stringify(failRun));
  const notify = await waitFor(async () =>
    provider.events.find((e) => e.type === 'notify.post' && /Harness failure/.test(e.data?.title ?? '')),
  );
  check('failure published notify.post', !!notify && notify.data.level === 'error', JSON.stringify(notify?.data));

  console.log('\noverlap + scheduling:');
  const slowJob = await caller.call('jobs.upsert', {
    name: 'Harness slow',
    enabled: true,
    trigger: { kind: 'manual' },
    action: { kind: 'shell', shell: { command: 'sleep 3' } },
  });
  const first = await caller.call('jobs.run', { id: slowJob.id });
  const second = await caller.call('jobs.run', { id: slowJob.id });
  check('second run-now while running is refused', first?.started === true && second?.started === false && /running/.test(second?.reason ?? ''), JSON.stringify(second));

  const daily = await caller.call('jobs.upsert', {
    name: 'Harness daily',
    enabled: true,
    trigger: { kind: 'daily', at: '09:00', days: [1, 2, 3, 4, 5] },
    action: { kind: 'shell', shell: { command: 'true' } },
  });
  let list = await caller.call('jobs.list', {});
  const dailyView = list.jobs.find((j) => j.id === daily.id);
  check('daily job schedules a future nextRunAt', (dailyView?.nextRunAt ?? 0) > Date.now(), JSON.stringify(dailyView));

  await caller.call('jobs.remove', { id: slowJob.id });
  list = await caller.call('jobs.list', {});
  check('remove drops the job', !list.jobs.some((j) => j.id === slowJob.id));

  console.log('\npersistence across a hub restart:');
  const beforeIds = list.jobs.map((j) => j.id).sort();
  caller.close();
  provider.close();
  hub.kill('SIGTERM');
  await sleep(700);
  startHub();
  const caller2 = await connectWithRetry('call2');
  const list2 = await caller2.call('jobs.list', {});
  const afterIds = list2.jobs.map((j) => j.id).sort();
  check('jobs survive the restart', JSON.stringify(afterIds) === JSON.stringify(beforeIds), `${beforeIds} vs ${afterIds}`);
  const histAfter = await caller2.call('jobs.history', { id: shellJob.id });
  check('run history survives the restart', (histAfter?.runs?.length ?? 0) >= 1 && /hello-from-the-harness/.test(histAfter.runs[0]?.detail ?? ''));
  caller2.close();

  console.log(failures === 0 ? '\nALL CHECKS PASSED' : `\n${failures} CHECK(S) FAILED`);
} finally {
  try {
    hub?.kill('SIGKILL');
  } catch {}
  fs.rmSync(tmp, { recursive: true, force: true });
}
process.exit(failures === 0 ? 0 : 1);
