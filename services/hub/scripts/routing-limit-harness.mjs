#!/usr/bin/env node
/**
 * Limit-aware routing harness.
 *
 * Starts a REAL scratch hub and points its existing --claudemon URL at the
 * fake claudemon in this directory. The fake walks through runtime usage
 * states that are not reproducible on demand against the real daemon.
 *
 * routing.select IS registered now, so every assertion below executes against
 * the real routing layer and none of them park. The PENDING machinery is kept
 * on purpose: it is what a branch that removes or renames routing.select looks
 * like, and it must be an obvious regression rather than a silent skip. Run
 * with ROUTING_HARNESS_REQUIRE_ROUTING=1 — as CI and the acceptance run do — to
 * make a parked assertion a FAILURE instead of a note.
 */
import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const hubDir = path.join(here, '..');
const fakeBin = path.join(here, 'fake-claudemon.mjs');
const TOKEN = 'routing-limit-harness-token';
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-routing-limit-harness-'));
const hubBin = path.join(tmp, 'hub');
const tokensFile = path.join(tmp, 'tokens.json');
const OPERATOR_TOKEN = 'routing-limit-harness-operator-token';
const decisionLog = path.join(tmp, 'routing-decisions.jsonl');
// The directory the ceiling below is keyed on. Resolved through realpath because
// the hub canonicalizes a spawn's cwd before the LEXICAL ancestor match, and on
// macOS os.tmpdir() is itself a symlink — a ceiling keyed on the unresolved
// spelling would never match and this harness would pass by never firing.
const CEILED_DIR = fs.realpathSync(fs.mkdtempSync(path.join(tmp, 'ceiled-')));
const requireRouting = process.env.ROUTING_HARNESS_REQUIRE_ROUTING === '1';

let failures = 0;
let activeChecks = 0;
const pendingChecks = [];
let fakeChild = null;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function check(name, cond, extra = '') {
  activeChecks++;
  const ok = !!cond;
  console.log(`${ok ? '  [PASS]' : '  [FAIL]'} ${name}${!ok && extra ? ` - ${extra}` : ''}`);
  if (!ok) failures++;
}

function pending(name, reason) {
  pendingChecks.push({ name, reason });
  console.log(`  [PENDING] ${name} - ${reason}`);
}

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const addr = srv.address();
      const port = typeof addr === 'object' && addr ? addr.port : 0;
      srv.close(() => resolve(port));
    });
  });
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

async function startFake(port) {
  const env = { ...process.env };
  delete env.NO_COLOR; // Avoid Node's FORCE_COLOR/NO_COLOR warning in child output.
  const child = spawn(process.execPath, [fakeBin, '--port', String(port)], {
    cwd: hubDir,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stderr.on('data', (d) => process.env.HARNESS_VERBOSE && process.stderr.write(d));

  return new Promise((resolve, reject) => {
    let settled = false;
    let stdout = '';
    const timer = setTimeout(() => {
      if (!settled) {
        child.kill('SIGKILL');
        reject(new Error('fake claudemon did not announce readiness'));
      }
    }, 8000);
    child.on('exit', (code, signal) => {
      if (!settled) {
        clearTimeout(timer);
        reject(new Error(`fake claudemon exited early: code=${code} signal=${signal}`));
      }
    });
    child.stdout.on('data', (d) => {
      if (process.env.HARNESS_VERBOSE) process.stdout.write(d);
      stdout += d.toString();
      for (const line of stdout.split(/\r?\n/)) {
        if (!line.trim()) continue;
        let msg;
        try {
          msg = JSON.parse(line);
        } catch {
          continue;
        }
        if (msg.type === 'listening' && msg.url) {
          settled = true;
          clearTimeout(timer);
          resolve({ child, url: msg.url });
          return;
        }
      }
    });
  });
}

let hub = null;
function startHub(hubPort, fakeURL) {
  const env = { ...process.env };
  delete env.WORKSPACER_PARENT_PID;
  hub = spawn(
    hubBin,
    [
      '-addr', `127.0.0.1:${hubPort}`,
      '-token', TOKEN,
      '-claudemon', fakeURL,
      '-brain-scope', 'off',
      '-jobs-file', '',
      '-layout-file', path.join(tmp, 'layout.json'),
      '-push-dir', path.join(tmp, 'push'),
      '-peers-file', path.join(tmp, 'peers.json'),
      // Keep the scratch hub off the developer's real
      // ~/.config/workspacer-hub/routing.yaml: the routing service SEEDS that
      // file on first run and re-reads it every tick, so without this the
      // harness's answers would depend on whatever the machine's own matrix
      // says. Pointing it at the scratch dir also exercises the seed path.
      '-routing-file', path.join(tmp, 'routing.yaml'),
      // A REAL tokens.json, because the binding half of this feature cannot be
      // exercised from the host token: that credential is the control plane and
      // is deliberately exempt from the caller-tier clamp. The ceiling clamp
      // applies to an operator record like any other.
      '-tokens-file', tokensFile,
    ],
    { env, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  hub.stderr.on('data', (d) => process.env.HARNESS_VERBOSE && process.stderr.write(d));
  hub.stdout.on('data', (d) => process.env.HARNESS_VERBOSE && process.stdout.write(d));
}

function connect(hubPort, name, token = TOKEN) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${hubPort}/bus?token=${token}`);
    let seq = 0;
    const pendingCalls = new Map();
    const handlers = new Map();
    const events = [];
    ws.onopen = () =>
      resolve({
        ws,
        events,
        subscribe(...topics) {
          ws.send(JSON.stringify({ op: 'subscribe', topics }));
        },
        register(methods) {
          ws.send(JSON.stringify({ op: 'register', methods }));
        },
        provide(method, fn) {
          handlers.set(method, fn);
        },
        call(method, params = {}) {
          return new Promise((res, rej) => {
            const id = `${name}-${++seq}`;
            pendingCalls.set(id, { res, rej });
            setTimeout(() => {
              if (pendingCalls.delete(id)) rej(new Error(`timeout: ${method}`));
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
    ws.onmessage = (m) => {
      const f = JSON.parse(m.data.toString());
      if (f.op === 'event') {
        events.push(f.event);
        return;
      }
      if (f.op === 'call') {
        const fn = handlers.get(f.method);
        const result = fn ? fn(f.params) : { ok: true };
        ws.send(JSON.stringify({ op: 'result', id: f.id, result }));
        return;
      }
      if (f.op !== 'result' && f.op !== 'error') return;
      const p = pendingCalls.get(f.id);
      if (!p) return;
      pendingCalls.delete(f.id);
      f.op === 'result' ? p.res(f.result) : p.rej(new Error(f.error));
    };
  });
}

async function connectWithRetry(hubPort, name, tries = 40, token = TOKEN) {
  for (let i = 0; i < tries; i++) {
    try {
      return await connect(hubPort, name, token);
    } catch {
      await sleep(250);
    }
  }
  throw new Error('hub never came up');
}

async function fakeJSON(fakeURL, route, init) {
  const res = await fetch(`${fakeURL}${route}`, {
    ...init,
    headers: { 'content-type': 'application/json', ...(init?.headers ?? {}) },
  });
  const text = await res.text();
  let body = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = text;
  }
  if (!res.ok) throw new Error(`${route} ${res.status}: ${text}`);
  return body;
}

const setScenario = (fakeURL, scenario, clearRequests = false) =>
  fakeJSON(fakeURL, '/usage/script', {
    method: 'POST',
    body: JSON.stringify({ scenario, clearRequests }),
  });

const clearFakeRequests = (fakeURL) =>
  fakeJSON(fakeURL, '/usage/requests/clear', { method: 'POST', body: '{}' });

const fakeRequestCount = async (fakeURL) => (await fakeJSON(fakeURL, '/usage/requests')).count ?? 0;

const setProviderAvailable = (fakeURL, provider, available) =>
  fakeJSON(fakeURL, '/providers/availability', {
    method: 'POST',
    body: JSON.stringify({ provider, available }),
  });

const modelProbeCount = async (fakeURL, provider) =>
  ((await fakeJSON(fakeURL, '/providers/probes')).probes ?? []).filter((p) => p.provider === provider).length;

const reasonsOf = (result) => (result?.reason ?? []).join(' | ');

function provider(report, name) {
  return report.providers.find((p) => p.provider === name);
}

function codexFive(report) {
  return provider(report, 'codex')?.accounts?.[0]?.windows?.five_hour;
}

function allStrings(v, out = []) {
  if (typeof v === 'string') out.push(v);
  else if (Array.isArray(v)) v.forEach((x) => allStrings(x, out));
  else if (v && typeof v === 'object') Object.values(v).forEach((x) => allStrings(x, out));
  return out;
}

function normalizedStrings(v) {
  return allStrings(v).map((s) => s.toLowerCase().replace(/[\s-]+/g, '_'));
}

function hasExactString(v, target) {
  return normalizedStrings(v).includes(target);
}

function serialized(v) {
  return JSON.stringify(v).toLowerCase();
}

function assertUnknownNamed(name, result) {
  check(`${name}: limit state is explicitly UNKNOWN`, hasExactString(result, 'unknown') || serialized(result).includes('unknown'), JSON.stringify(result));
  check(`${name}: no GREEN/healthy default leaked`, !hasExactString(result, 'green') && !hasExactString(result, 'healthy'), JSON.stringify(result));
}

function assertNoStaleModes(name, result) {
  check(`${name}: stale/non-current window did not choose CONSERVE`, !hasExactString(result, 'conserve'), JSON.stringify(result));
  check(`${name}: stale/non-current window did not choose SPEND_DOWN`, !hasExactString(result, 'spend_down'), JSON.stringify(result));
}

function routingRequest(scenario, providerName = 'codex') {
  return {
    ticketId: `routing-limit-harness-${scenario}`,
    role: 'scout',
    difficulty: 'medium',
    risk: 'medium',
    decisionDensity: 'medium',
    previousProvider: null,
    requireIndependentFamily: false,
    profile: 'mixed',
    provider: providerName,
    preferredProvider: providerName,
    account: '',
    profileId: '',
    cwd: tmp,
    forecastDemandBeforeResetPct: 0,
  };
}

async function maybeCallRouting(caller, scenario, request) {
  try {
    return { ok: true, result: await caller.call('routing.select', request) };
  } catch (err) {
    const msg = String(err?.message ?? err);
    if (/no provider for routing\.select\b/.test(msg)) return { ok: false, missingRouting: true, error: msg };
    return { ok: false, error: msg };
  }
}

async function runRoutingCases(caller, fakeURL) {
  console.log('\nrouting.select assertions:');
  const cases = [
    {
      scenario: 'stale-codex',
      provider: 'codex',
      checks(result) {
        assertNoStaleModes('stale-codex', result);
        check('stale-codex: response states the stale Codex bucket as UNKNOWN', serialized(result).includes('codex') && serialized(result).includes('unknown'), JSON.stringify(result));
      },
    },
    {
      scenario: 'reset-now',
      provider: 'codex',
      checks(result) {
        assertNoStaleModes('reset-now', result);
        check('reset-now: boundary reset is UNKNOWN, not a current percentage', serialized(result).includes('unknown'), JSON.stringify(result));
      },
    },
    {
      scenario: 'copilot-403',
      provider: 'copilot',
      checks(result) {
        assertUnknownNamed('copilot-403', result);
        check('copilot-403: response mentions the requested provider', serialized(result).includes('copilot'), JSON.stringify(result));
      },
    },
    {
      scenario: 'absent-opencode-pi',
      provider: 'opencode',
      checks(result) {
        assertUnknownNamed('absent-opencode-pi', result);
        check('absent-opencode-pi: response mentions the absent requested provider', serialized(result).includes('opencode'), JSON.stringify(result));
      },
    },
  ];

  let routingKnownMissing = false;
  for (const c of cases) {
    await setScenario(fakeURL, c.scenario, true);
    const before = await fakeRequestCount(fakeURL);
    const eventStart = caller.events.length;
    if (routingKnownMissing) {
      pending(`${c.scenario}: routing decision assertions`, 'routing.select is not registered on this branch');
      continue;
    }
    const call = await maybeCallRouting(caller, c.scenario, routingRequest(c.scenario, c.provider));
    if (call.missingRouting) {
      routingKnownMissing = true;
      pending(`${c.scenario}: routing decision assertions`, 'routing.select is not registered on this branch');
      continue;
    }
    if (!call.ok) {
      check(`${c.scenario}: routing.select returned a result`, false, call.error);
      continue;
    }
    const sawUsage = await waitFor(async () => (await fakeRequestCount(fakeURL)) > before, 5000);
    check(`${c.scenario}: hub read fake claudemon /usage/report`, !!sawUsage, 'routing.select returned without touching the fake usage report');
    c.checks({ result: call.result, events: caller.events.slice(eventStart) });
  }

  if (routingKnownMissing) {
    for (const c of cases.slice(1)) {
      if (!pendingChecks.some((p) => p.name.startsWith(c.scenario))) {
        pending(`${c.scenario}: routing decision assertions`, 'routing.select is not registered on this branch');
      }
    }
  }
}

// runPacingAssertions: WINDOW PROGRESS, against a live hub.
//
// The claim pacing makes cannot be checked from a used-percentage alone, which
// is exactly why it needs a runtime harness: every scenario below is a CURRENT
// window with ordinary health, and the only thing separating them is how far
// through the window the reading is. If the window LENGTH ever stops arriving
// from the daemon — which is the fragile half, since the Anthropic endpoint
// never states it and the daemon asserts it — every one of these silently
// reverts to "unknown" and the checks here fail rather than passing quietly.
async function runPacingAssertions(caller, fakeURL) {
  console.log('\nwindow-progress (pacing) assertions:');

  const ask = async (scenario, ticket) => {
    await setScenario(fakeURL, scenario, true);
    const eventStart = caller.events.length;
    const result = await caller.call('routing.select', {
      ...routingRequest(scenario, 'claude'),
      role: 'implementer',
      ticketId: ticket,
    });
    return { result, events: caller.events.slice(eventStart) };
  };

  // 1. The Anthropic window LENGTHS reach the hub, which is requirement one of
  //    this feature and the precondition for everything below.
  const report = await fakeJSON(fakeURL, '/usage/report');
  const claudeWindows = provider(report, 'claude')?.accounts?.[0]?.windows ?? {};
  check('claude 5h window carries its 300-minute length', claudeWindows.five_hour?.window_minutes === 300, JSON.stringify(claudeWindows.five_hour));
  check('claude 7d window carries its 10080-minute length', claudeWindows.seven_day?.window_minutes === 10080, JSON.stringify(claudeWindows.seven_day));
  check('claude monthly window carries NO length (a calendar month is not fixed)', claudeWindows.monthly?.window_minutes == null, JSON.stringify(claudeWindows.monthly));

  // 2. On pace: a healthy, on-schedule provider is paced and left alone.
  const healthy = await ask('healthy-current', 'HARNESS-PACE-OK');
  const okPace = healthy.result?.capacity?.pace;
  check('a current claude window is PACED at all (the length arrived end to end)', okPace?.known === true, JSON.stringify(okPace));
  check('an on-schedule provider is on_track', okPace?.state === 'on_track', JSON.stringify(okPace));
  check('an on-schedule provider is not conserved by pace', healthy.result?.mode !== 'conserve', JSON.stringify(healthy.result?.reason));

  // 3. Overspending: the case the used-percentage ladder cannot see. 80% of a
  //    five-hour window with half the window left is YELLOW health and a fleet
  //    that will be out of allowance within the hour.
  const over = await ask('claude-overpace', 'HARNESS-PACE-OVER');
  const pace = over.result?.capacity?.pace;
  check('an over-curve window is OVERSPENDING', pace?.state === 'overspending', JSON.stringify(pace));
  check('the binding window is named', pace?.window === 'five_hour', JSON.stringify(pace));
  check('the ratio is above the shipped 1.25 band', typeof pace?.ratio === 'number' && pace.ratio > 1.25, JSON.stringify(pace));
  check('health alone would NOT have conserved this (it is yellow, not red)', over.result?.capacity?.health === 'yellow', JSON.stringify(over.result?.capacity?.health));
  check('the decision is CONSERVE', over.result?.mode === 'conserve', JSON.stringify(over.result?.reason));
  check(
    'the conserve is EXPLAINED in the answer, not merely asserted',
    (over.result?.reason ?? []).some((r) => r.includes('faster than it refills')),
    JSON.stringify(over.result?.reason),
  );
  check('every window\'s pace is reported, not only the binding one', Array.isArray(over.result?.capacity?.paceWindows) && over.result.capacity.paceWindows.length >= 2, JSON.stringify(over.result?.capacity?.paceWindows));

  // 4. The event plane carries the pace, so a fleet display can caption a mode
  //    change without parsing prose.
  const ev = await waitFor(
    async () => over.events.concat(caller.events.slice(-20)).find((e) => e?.type === 'routing.decision' && e?.data?.decisionId === over.result?.decisionId),
    5000,
  );
  check('the routing.decision event carries the pace state', ev?.data?.pace === 'overspending', JSON.stringify(ev?.data));
  check('the routing.decision event carries the ratio and the window', ev?.data?.paceRatio > 1.25 && ev?.data?.paceWindow === 'five_hour', JSON.stringify(ev?.data));

  // 5. The middle band: ahead of the curve blocks a spend-down WITHOUT
  //    conserving. Before pacing this scenario is a textbook spend-down.
  const blocked = await ask('claude-pace-blocks-spend-down', 'HARNESS-PACE-BLOCK');
  const blockPace = blocked.result?.capacity?.pace;
  check('a week running ahead of the curve is AHEAD, not overspending', blockPace?.state === 'ahead', JSON.stringify(blockPace));
  check('the spend-down is blocked', blocked.result?.mode !== 'spend_down', JSON.stringify(blocked.result?.reason));
  check('blocking a spend-down is not conserving', blocked.result?.mode === 'normal', JSON.stringify(blocked.result?.reason));
  check(
    'the block is explained',
    (blocked.result?.reason ?? []).some((r) => r.includes('not going to expire unused')),
    JSON.stringify(blocked.result?.reason),
  );

  // 6. A provider with no window length is untouched: copilot publishes nothing
  //    and must stay conservative rather than being paced against a
  //    denominator nobody reported.
  await setScenario(fakeURL, 'copilot-403', true);
  const dark = await caller.call('routing.select', {
    ...routingRequest('copilot-pace', 'copilot'),
    role: 'implementer',
  });
  check('a provider that publishes no window is pace-UNKNOWN', dark?.capacity?.pace?.known !== true, JSON.stringify(dark?.capacity?.pace));
  check('and it is still explained rather than silently dropped', typeof dark?.capacity?.pace?.because === 'string' && dark.capacity.pace.because.length > 0, JSON.stringify(dark?.capacity?.pace));
  check('an unpaceable provider is not conserved by pace', !(dark?.reason ?? []).some((r) => r.includes('faster than it refills')), JSON.stringify(dark?.reason));
}

// ---------------------------------------------------------------------------
// THE BINDING HALF
// ---------------------------------------------------------------------------

// seedFixtures writes the two files the hub reads before it starts: an operator
// token record (the host token is the control plane and is exempt from the
// caller-tier clamp, so it cannot exercise the binding half) and a routing.yaml
// whose `ceilings:` block caps ONE directory. The rest of the matrix is the
// shipped default, merged underneath.
function seedFixtures() {
  fs.writeFileSync(
    tokensFile,
    JSON.stringify([{ token: OPERATOR_TOKEN, scope: 'operator', label: 'routing harness', created: '2026-08-30T00:00:00Z' }]),
    { mode: 0o600 },
  );
  fs.writeFileSync(
    path.join(tmp, 'routing.yaml'),
    [
      'ceilings:',
      '  default: { max_capability: frontier_plus, max_tool_scope: operator }',
      `  ${CEILED_DIR}: { max_capability: balanced, max_tool_scope: triage }`,
      // --- the shapes slice 2's cases need, and that the shipped file has no
      //     reason to carry. Everything else is the shipped default, merged
      //     underneath, so the cases above keep measuring what they measured.
      //
      // conserve moves the JUDGE across providers (frontier_plus is claude,
      // cheap is codex) and cheap's own codex row is switched off — which is
      // what makes the mode shift and the fallover walk compose on one answer.
      'mode_shifts:',
      '  conserve:',
      '    judge: cheap',
      'profiles:',
      '  mixed:',
      '    cheap:',
      '      enabled: false',
      '      alternatives:',
      '        - { provider: copilot, model: gpt-5-mini }',
      // A THREE-CANDIDATE capability whose MIDDLE entry is disabled, so a walk
      // has something to step over and name on its way past.
      '    deep_reviewer:',
      '      alternatives:',
      '        - { provider: codex, model: gpt-5.6-sol, effort: high, fresh: true, enabled: false }',
      '        - { provider: copilot, model: gpt-5-mini, fresh: true }',
      '',
    ].join('\n'),
    { mode: 0o600 },
  );
  // The seed marker, so the service records the file as the user's rather than
  // overwriting it with the shipped default on first run.
  fs.writeFileSync(path.join(tmp, 'routing.yaml.seeded'), '{"seededVersion":1}\n');
}

function readDecisionLog() {
  let raw = '';
  try {
    raw = fs.readFileSync(decisionLog, 'utf8');
  } catch {
    return [];
  }
  return raw
    .split(/\r?\n/)
    .filter((l) => l.trim())
    .map((l) => {
      try {
        return JSON.parse(l);
      } catch {
        return null;
      }
    })
    .filter(Boolean);
}

// runDecisionRecordAssertions: the routing.decision event and the decision log,
// against a LIVE hub. Both are what make the feature auditable, and the log is
// the data that later lets forecast_weights be calibrated into a real share of
// an allowance rather than the unitless weights it carries today.
async function runDecisionRecordAssertions(caller, fakeURL) {
  console.log('\nrouting.decision event + decision log:');
  await setScenario(fakeURL, 'healthy-current', true);
  const before = caller.events.length;
  const result = await caller.call('routing.select', {
    ...routingRequest('record', 'codex'),
    role: 'implementer',
    ticketId: 'HARNESS-REC-1',
  });
  const decisionId = result?.decisionId;
  check('routing.select stamps a decisionId', typeof decisionId === 'string' && decisionId.startsWith('rd_'), JSON.stringify(result));

  const ev = await waitFor(
    async () => caller.events.slice(before).find((e) => e?.type === 'routing.decision' && e?.data?.decisionId === decisionId),
    5000,
  );
  check('a routing.decision event is published for the answer', !!ev, JSON.stringify(caller.events.slice(before)));
  if (ev) {
    check('the event carries the model and mode the answer chose', ev.data.model === result.model && ev.data.mode === result.mode, JSON.stringify(ev.data));
    check('the event does NOT carry the caller cwd (open-by-decision topic)', ev.data.cwd === undefined, JSON.stringify(ev.data));
    check('the event names the ticket it was asked about', ev.data.ticketId === 'HARNESS-REC-1', JSON.stringify(ev.data));
  }

  const rows = await waitFor(async () => {
    const all = readDecisionLog();
    return all.some((r) => r.kind === 'decision' && r.decisionId === decisionId) ? all : null;
  }, 5000);
  check('the decision is appended to routing-decisions.jsonl', !!rows, `no decision row for ${decisionId} in ${decisionLog}`);
  if (rows) {
    const row = rows.find((r) => r.kind === 'decision' && r.decisionId === decisionId);
    check('the logged decision carries the model it answered', row?.decision?.model === result.model, JSON.stringify(row));
    check('the logged decision is timestamped', typeof row?.at === 'string' && row.at.length > 0, JSON.stringify(row));
  }
  return decisionId;
}

// runCeilingAwareSelectAssertions: routing.select CONSULTS the ceiling before it
// answers, so it can never advise a model the spawn gate will then take away.
//
// Until it did, the system contradicted itself once per capped dispatch: select
// answered Fable for a judge, the gate stripped the model, and the worker
// arrived as an unexplained downgrade. Proving it needs a LIVE hub because the
// half that could break is the canonicalization — routing.select and the gate
// must resolve the caller's cwd the same way, or they cap different directories.
async function runCeilingAwareSelectAssertions(caller) {
  console.log('\nrouting.select is ceiling-aware:');

  // Inside the capped tree: the answer must already be at or under the cap.
  const capped = await caller.call('routing.select', {
    ...routingRequest('ceiling', 'codex'),
    role: 'implementer',
    provider: '',
    preferredProvider: '',
    cwd: CEILED_DIR,
  });
  check('a select inside a capped directory still answers something spawnable', capped?.eligible === true, JSON.stringify(capped));
  check('the answer is capped to the directory ceiling', capped?.capability === 'balanced', JSON.stringify(capped));
  check('the answer still records what the ROLE asked for', capped?.baseCapability === 'frontier', JSON.stringify(capped));
  check('the decision reports the ceiling it was resolved under', capped?.ceiling?.capabilityRefused === true, JSON.stringify(capped?.ceiling));
  check('the cap is EXPLAINED in the answer, not only in a log', (capped?.reason ?? []).some((r) => r.includes('ceilings.')), JSON.stringify(capped?.reason));

  // THE WHOLE POINT: the gate would not refuse what select just advised.
  const provider = capped?.provider;
  const model = capped?.model;
  check('the advised tuple is complete enough to dispatch', !!provider && !!model, JSON.stringify(capped));

  // Outside the capped tree the SAME role is not capped, so the check above is
  // measuring the ceiling rather than a clamp that fires everywhere.
  const free = await caller.call('routing.select', {
    ...routingRequest('ceiling-free', 'codex'),
    role: 'implementer',
    provider: '',
    preferredProvider: '',
    cwd: tmp,
  });
  // Not pinned to a literal capability: this harness runs its scenarios under
  // SPEND_DOWN, which legitimately promotes the implementer to frontier_max. The
  // claim is that the CEILING did not bite here, which is what makes the capped
  // case above a measurement of the ceiling rather than of a clamp firing
  // everywhere.
  check('the same role outside the capped tree is not capped at all', !free?.ceiling?.capabilityRefused, JSON.stringify(free?.ceiling));
  check('and it keeps a capability above the capped tree\'s ceiling', free?.capability !== 'balanced', JSON.stringify(free));
  // THE ORDER MATTERS: the mode shift moves the capability first and the ceiling
  // caps the RESULT, so a spend_down promotion cannot be used to climb past a cap.
  check('a mode-shift promotion is itself subject to the ceiling', capped?.capability === 'balanced' && capped?.reason?.some((r) => r.includes('mode_shifts')) !== undefined, JSON.stringify(capped?.reason));

  // A SYMLINK naming the capped directory is capped too — the same
  // canonicalizing walk the gate uses, or the two would disagree about which
  // directory a decision is for.
  const link = path.join(tmp, 'select-ceiled-link');
  try {
    fs.symlinkSync(CEILED_DIR, link);
  } catch {
    /* already there from a previous case */
  }
  const viaLink = await caller.call('routing.select', {
    ...routingRequest('ceiling-link', 'codex'),
    role: 'implementer',
    provider: '',
    preferredProvider: '',
    cwd: link,
  });
  check('a SYMLINK to the capped directory is capped by routing.select too', viaLink?.capability === 'balanced', JSON.stringify(viaLink));
}

// runSpawnBindingAssertions is THE acceptance test for this slice: a real
// agents.spawn, from a real scoped operator credential, through the real hub
// router, into a real registered provider — and the ceiling in routing.yaml
// takes the escalation away before the provider ever sees it.
//
// Until this existed, routing was ADVISORY: a manager could ask routing.select,
// ignore the answer, and spawn whatever it liked.
async function runSpawnBindingAssertions(hubPort, decisionId) {
  console.log('\nagents.spawn ceiling enforcement:');
  const provider = await connectWithRetry(hubPort, 'spawn-provider');
  const seen = [];
  provider.provide('agents.spawn', (params) => {
    seen.push(params);
    return { sessionId: `harness-sess-${seen.length}` };
  });
  provider.register(['agents.spawn']);
  await sleep(300); // let the registration settle

  const operator = await connectWithRetry(hubPort, 'operator', 40, OPERATOR_TOKEN);

  // 1. ABOVE the ceiling, inside the capped directory.
  const over = await operator.call('agents.spawn', {
    cwd: CEILED_DIR,
    capability: 'frontier_plus',
    model: 'fable',
    effort: 'high',
    toolScope: 'operator',
    role: 'judge',
    decisionId,
  });
  const got = seen[seen.length - 1] ?? {};
  check('the spawn reached the provider at all', !!over?.sessionId, JSON.stringify(over));
  check('capability was CLAMPED to the directory ceiling', got.capability === 'balanced', JSON.stringify(got));
  check('the model the refused capability chose did not survive', got.model !== 'fable', JSON.stringify(got));
  // STRONGER THAN "dropped", and deliberately. Dropping `model` leaves the
  // PROVIDER to resolve its own configured default, one layer below anything the
  // ceiling can see, so the clamp must NAME what the permitted capability
  // resolves to and leave no hole for a default to fill. (That default is
  // `opus[1m]`, which the named-model arm can read now as well — see 2c.)
  check('the clamp left no hole for a provider default to fill', typeof got.model === 'string' && got.model.length > 0, JSON.stringify(got));
  check('the replacement model is what the PERMITTED capability resolves to', got.model === 'sonnet', JSON.stringify(got));
  check('the replacement carries its own effort', got.effort === 'high', JSON.stringify(got));
  check('the clamp did not swap the harness the spawn was for', got.provider === 'claude', JSON.stringify(got));
  check('the tool tier was clamped to the directory ceiling', got.toolScope === 'triage', JSON.stringify(got));
  check(
    'the downgrade is named in escalationScrubbed (no silent downgrades)',
    Array.isArray(got.escalationScrubbed) &&
      ['capability', 'model', 'effort', 'toolScope'].every((f) => got.escalationScrubbed.includes(f)),
    JSON.stringify(got),
  );
  check('the recorded routing metadata rode through untouched', got.role === 'judge' && got.decisionId === decisionId, JSON.stringify(got));

  // 2. UNDER the ceiling, same directory: nothing is taken.
  await operator.call('agents.spawn', { cwd: CEILED_DIR, capability: 'cheap', model: 'gpt-5.6-luna', toolScope: 'triage' });
  const under = seen[seen.length - 1] ?? {};
  check('a spawn under the ceiling keeps its capability', under.capability === 'cheap', JSON.stringify(under));
  check('a spawn under the ceiling keeps its model', under.model === 'gpt-5.6-luna', JSON.stringify(under));
  check('a spawn under the ceiling reports nothing scrubbed', under.escalationScrubbed === undefined, JSON.stringify(under));

  // 2c. THE CONTEXT-WINDOW SUFFIX, judged rather than waved through. `opus[1m]`
  //     is `opus` with a 1M window instead of the standard 200K, and it is the
  //     desktop's shipped `claude.defaultModel` — so while the named-model arm
  //     compared raw strings, the strongest model on the Claude path was
  //     reachable from inside a capped directory by spelling it the way the
  //     matrix does not, or by omitting `model` entirely and letting the
  //     provider fill that default in below the gate. The arm now compares on
  //     the model with the window suffix taken off.
  await operator.call('agents.spawn', { cwd: CEILED_DIR, provider: 'claude', model: 'opus[1m]' });
  const suffixed = seen[seen.length - 1] ?? {};
  check('a window-suffixed model is judged by the named-model arm', suffixed.capability === 'balanced', JSON.stringify(suffixed));
  check('the suffixed model did not survive the clamp', suffixed.model !== 'opus[1m]', JSON.stringify(suffixed));
  check('its replacement is what the PERMITTED capability resolves to', suffixed.model === 'sonnet', JSON.stringify(suffixed));
  check('the refused 1M request did not ride along onto the replacement', !String(suffixed.model ?? '').includes('[1m]'), JSON.stringify(suffixed));
  check(
    'the suffixed clamp is named in escalationScrubbed too',
    Array.isArray(suffixed.escalationScrubbed) && suffixed.escalationScrubbed.includes('model'),
    JSON.stringify(suffixed),
  );

  // 2b. THE SYMLINK. CeilingFor is a LEXICAL ancestor match, so a spawn whose
  //     cwd merely NAMES the capped directory through a link must still be
  //     judged by it — the hub canonicalizes before the lookup, and if it ever
  //     stops, this is the case that walks around every per-directory ceiling.
  const link = path.join(tmp, 'ceiled-link');
  try {
    fs.symlinkSync(CEILED_DIR, link);
    await operator.call('agents.spawn', { cwd: link, capability: 'frontier_plus', model: 'fable', toolScope: 'operator' });
    const viaLink = seen[seen.length - 1] ?? {};
    check('a SYMLINK to the capped directory does not walk around its ceiling', viaLink.capability === 'balanced' && viaLink.model === 'sonnet', JSON.stringify(viaLink));
    check('the symlinked spawn is also tier-clamped', viaLink.toolScope === 'triage', JSON.stringify(viaLink));
  } catch (err) {
    check('symlink case ran', false, `could not create ${link}: ${err?.message ?? err}`);
  }

  // 3. OUTSIDE the capped tree: the permissive default ceiling applies, so the
  //    same request that was refused above is admitted here. Without this the
  //    clamp could be firing everywhere and the case above would not notice.
  await operator.call('agents.spawn', { cwd: tmp, capability: 'frontier_plus', model: 'fable', toolScope: 'operator' });
  const outside = seen[seen.length - 1] ?? {};
  check('the SAME request outside the capped tree is admitted', outside.capability === 'frontier_plus' && outside.model === 'fable', JSON.stringify(outside));

  // 3b. AND THE SUFFIX SURVIVES TO THE PROVIDER. This is the half that would be
  //     worse to get wrong than the gap it closes: the ceiling normalizes the
  //     window suffix away for its COMPARISON only, so a spawn the ceiling
  //     admits has to arrive at the provider spelled exactly as it was sent.
  //     Strip it here and every dispatch silently drops from 1M to 200K, and
  //     nothing surfaces that until an agent runs out of room.
  await operator.call('agents.spawn', { cwd: tmp, provider: 'claude', model: 'opus[1m]' });
  const kept = seen[seen.length - 1] ?? {};
  check('an admitted model reaches the provider carrying its window request', kept.model === 'opus[1m]', JSON.stringify(kept));
  check('and the admitted spawn reports nothing scrubbed', kept.escalationScrubbed === undefined, JSON.stringify(kept));

  // 5. FRESHNESS. The shipped matrix marks reviewer, deep_reviewer and
  //    frontier_plus `fresh: true`, and a fresh worker that inherits the
  //    previous agent's conversation is not fresh. So a spawn declaring that
  //    work and carrying a `resumeSessionId` is REFUSED — not stripped, because
  //    stripping would start a new session the caller believes is a
  //    continuation. Asserted against the live hub for the same reason the
  //    ceiling is: the guarantee is about what the provider is handed.
  const refused = async (params) => {
    const before = seen.length;
    let err = null;
    try {
      await operator.call('agents.spawn', params);
    } catch (e) {
      err = e?.message ?? String(e);
    }
    return { err, reached: seen.length > before };
  };

  const reviewerResume = await refused({
    cwd: tmp,
    role: 'reviewer',
    resumeSessionId: 'implementer-sess-1',
  });
  check('a reviewer asking to RESUME a session is refused', typeof reviewerResume.err === 'string', JSON.stringify(reviewerResume));
  check(
    'the refusal names the session it would have inherited',
    (reviewerResume.err ?? '').includes('implementer-sess-1'),
    reviewerResume.err ?? '',
  );
  check(
    'the refusal names `fresh` as the reason, in the ANSWER and not only a log',
    /fresh/.test(reviewerResume.err ?? ''),
    reviewerResume.err ?? '',
  );
  check('the refused reviewer never reached the provider', !reviewerResume.reached, JSON.stringify(reviewerResume));

  // A capability declared DIRECTLY, with no role at all — the shape a routed
  // dispatch has when it copies the decision's capability onto the spawn.
  const capResume = await refused({ cwd: tmp, capability: 'deep_reviewer', resumeSessionId: 'sess-2' });
  check('a fresh CAPABILITY declared without a role is refused too', typeof capResume.err === 'string', JSON.stringify(capResume));

  // …and inside the capped directory, where the ceiling also bites, so the two
  // refusals are proven not to be the same mechanism wearing two names.
  const bothRefused = await refused({
    cwd: CEILED_DIR,
    role: 'reviewer',
    capability: 'frontier_plus',
    resumeSessionId: 'sess-3',
  });
  check('the freshness refusal fires inside a capped directory as well', typeof bothRefused.err === 'string', JSON.stringify(bothRefused));

  // THE OTHER SIDE, and the reason the cases above measure the rule rather than
  // a hub that refuses every resume.
  await operator.call('agents.spawn', { cwd: tmp, role: 'implementer', resumeSessionId: 'sess-ok-1' });
  const ordinary = seen[seen.length - 1] ?? {};
  check('an implementer keeps its resume', ordinary.resumeSessionId === 'sess-ok-1', JSON.stringify(ordinary));

  await operator.call('agents.spawn', { cwd: tmp, resumeSessionId: 'sess-ok-2' });
  const unlabelled = seen[seen.length - 1] ?? {};
  check('a spawn declaring no role and no capability keeps its resume', unlabelled.resumeSessionId === 'sess-ok-2', JSON.stringify(unlabelled));

  await operator.call('agents.spawn', { cwd: tmp, role: 'reviewer', capability: 'deep_reviewer' });
  const freshStart = seen[seen.length - 1] ?? {};
  check('a reviewer starting a NEW session is admitted', freshStart.role === 'reviewer' && freshStart.capability === 'deep_reviewer', JSON.stringify(freshStart));

  // 4. The spawn is recorded, and it joins the decision that produced it.
  const joined = await waitFor(async () => {
    const rows = readDecisionLog();
    return rows.find((r) => r.kind === 'spawn' && r.decisionId === decisionId) ?? null;
  }, 5000);
  check('the clamped spawn is appended to the decision log', !!joined, `no spawn row for ${decisionId}`);
  if (joined) {
    check('the spawn row records the ceiling that matched', joined.spawn?.ceiling?.key === CEILED_DIR, JSON.stringify(joined.spawn?.ceiling));
    check('the spawn row records what was taken', Array.isArray(joined.spawn?.scrubbed) && joined.spawn.scrubbed.includes('model'), JSON.stringify(joined.spawn));
    check('the spawn row records the CANONICAL cwd the ceiling was looked up on', joined.spawn?.cwd === CEILED_DIR, JSON.stringify(joined.spawn));
    check('the spawn row records the caller tier, not its token', joined.spawn?.callerScope === 'operator' && !JSON.stringify(joined.spawn).includes(OPERATOR_TOKEN), JSON.stringify(joined.spawn));
  }

  provider.close();
  operator.close();
}


// ---------------------------------------------------------------------------
// SLICE 2: EFFORT STEPPING, THE COMPOSED WALK, AND LIVE AVAILABILITY
//
// Every case below runs against the live hub for the same reason the pacing
// ones do: the halves that can break are the ones that only exist end to end —
// the matrix file's new keys surviving the seed/merge/reload path, the mode's
// step reaching the row a fallover actually chose, and the availability map
// being built at the edge and injected into a Select that cannot probe.
// ---------------------------------------------------------------------------

async function runSlice2RoutingAssertions(caller, fakeURL) {
  console.log('\nslice 2: fallover shapes and effort stepping:');

  const ask = async (scenario, request) => {
    await setScenario(fakeURL, scenario, true);
    const eventStart = caller.events.length;
    const result = await caller.call('routing.select', {
      ...routingRequest(scenario, ''),
      provider: '',
      preferredProvider: '',
      ...request,
    });
    const ev = await waitFor(
      async () => caller.events.slice(eventStart).find((e) => e?.type === 'routing.decision' && e?.data?.decisionId === result?.decisionId),
      5000,
    );
    return { result, event: ev?.data };
  };

  // 1. A RED PRIMARY, ACROSS PROVIDERS, ON THE EVENT PLANE. codex is out of
  //    allowance, mixed's frontier alternative is claude, and the published
  //    event has to describe the provider that will actually run the work.
  const red = await ask('codex-red', { role: 'implementer' });
  check('codex-red: the answer falls over to the claude alternative', red.result?.provider === 'claude' && red.result?.model === 'opus', JSON.stringify(red.result));
  check('codex-red: the capability is unchanged — a fallover moves the PROVIDER', red.result?.capability === 'frontier', JSON.stringify(red.result?.capability));
  check('codex-red: the subject capacity is still codex\'s own RED reading', red.result?.capacity?.provider === 'codex' && red.result?.capacity?.health === 'red', JSON.stringify(red.result?.capacity?.health));
  check('codex-red: the event names the primary it passed over', red.event?.fellOverFrom?.provider === 'codex', JSON.stringify(red.event?.fellOverFrom));
  check('codex-red: the event health describes CLAUDE, the provider about to run it', red.event?.health === 'green' && red.event?.provider === 'claude', JSON.stringify({ health: red.event?.health, provider: red.event?.provider }));

  // 5a. CONSERVE TRIMS THE EFFORT of the row it landed on — the same model,
  //     thinking one notch less, which is the move that had no spelling before.
  check('codex-red: conserve steps the landing row down one rung', red.result?.effort === 'medium', JSON.stringify(red.result?.effort));
  check('codex-red: the step is reported as a structured from/to', red.result?.effortStep?.from === 'high' && red.result?.effortStep?.to === 'medium', JSON.stringify(red.result?.effortStep));
  check('codex-red: the step rides the routing.decision event', red.event?.effortStep?.to === 'medium' && red.event?.effort === 'medium', JSON.stringify(red.event?.effortStep));
  check('codex-red: the step is explained, not merely applied', /steps down the claude ladder/.test(red.result?.effortStep?.why ?? ''), red.result?.effortStep?.why ?? '');

  // 5b. AND IT LEAVES THE CHEAP TIERS ALONE. Same scenario, same conserve, a
  //     role whose capability is not on the allow-list: a scout on Sonnet at
  //     `medium` is a worse scout rather than a cheaper one.
  const balanced = await ask('codex-red', { role: 'fixer' });
  check('codex-red: a balanced tier is NOT stepped', balanced.result?.capability === 'balanced' && balanced.result?.effort === 'high', JSON.stringify({ capability: balanced.result?.capability, effort: balanced.result?.effort }));
  check('codex-red: and it says why it was left alone', /effort_step_capabilities/.test(balanced.result?.effortStep?.why ?? ''), balanced.result?.effortStep?.why ?? '');

  // 2. A SKIPPED MIDDLE CANDIDATE. The walk steps over a disabled alternative
  //    and NAMES it — a silent skip reads as "there was no alternative".
  const skipped = await ask('claude-overpace', { role: 'deep_reviewer' });
  check('skipped-middle: the walk lands on the third candidate', skipped.result?.provider === 'copilot' && skipped.result?.model === 'gpt-5-mini', JSON.stringify(skipped.result));
  check('skipped-middle: the disabled middle entry is named in the reasons', /alternative codex gpt-5\.6-sol[^|]*was skipped too: the entry is explicitly disabled/.test(reasonsOf(skipped.result)), reasonsOf(skipped.result));
  check('skipped-middle: the primary it fell over FROM is the claude one', skipped.result?.fellOverFrom?.provider === 'claude', JSON.stringify(skipped.result?.fellOverFrom));

  // 3. A PINNED PROVIDER IS SERVED FROM ITS OWN PROFILE. Asking for claude
  //    under `mixed` must take mixed's own cross-family pairing, not borrow
  //    another profile's whole opinion about effort and freshness.
  const pinned = await ask('healthy-current', { role: 'implementer', provider: 'claude', preferredProvider: 'claude' });
  check('pinned-provider: the answer is on the provider that was asked for', pinned.result?.provider === 'claude', JSON.stringify(pinned.result));
  check('pinned-provider: it did NOT borrow another profile\'s pairing', !/took the pairing from profile/.test(reasonsOf(pinned.result)), reasonsOf(pinned.result));

  // 4. INDEPENDENT FAMILY: a preference the walk honours, and an honest report
  //    when capacity will not allow it.
  const independent = await ask('healthy-current', {
    role: 'reviewer', previousProvider: 'codex', requireIndependentFamily: true,
  });
  check('independent-family: the reviewer lands on a different family from the implementer', independent.result?.provider === 'claude' && independent.result?.independentFamily === true, JSON.stringify({ provider: independent.result?.provider, independentFamily: independent.result?.independentFamily }));
  check('independent-family: the walk says it tried the independent candidates first', /prefers a different one first/.test(reasonsOf(independent.result)), reasonsOf(independent.result));

  const sameFamily = await ask('claude-overpace', {
    role: 'reviewer', previousProvider: 'codex', requireIndependentFamily: true,
  });
  check('same-family fallback: a conserving claude sends the reviewer back to codex', sameFamily.result?.provider === 'codex', JSON.stringify(sameFamily.result));
  check('same-family fallback: independence is reported HONESTLY as lost', sameFamily.result?.independentFamily === false, JSON.stringify(sameFamily.result?.independentFamily));
  check('same-family fallback: and the answer says it could not be arranged', /independent family was REQUIRED and could not be arranged/.test(reasonsOf(sameFamily.result)), reasonsOf(sameFamily.result));
  check('same-family fallback: `fresh: true` is what still carries independence', sameFamily.result?.fresh === true, JSON.stringify(sameFamily.result?.fresh));

  // 6. SPEND_DOWN STEPS UP, and stops at two different limits.
  const spend = await ask('healthy-current', { role: 'supervisor', provider: 'codex', preferredProvider: 'codex' });
  check('spend_down: the mode is actually spend_down', spend.result?.mode === 'spend_down', JSON.stringify(spend.result?.reason));
  check('spend_down: an unpromoted role steps UP one rung', spend.result?.effort === 'xhigh' && spend.result?.effortStep?.from === 'high', JSON.stringify(spend.result?.effortStep));

  const promoted = await ask('healthy-current', { role: 'implementer', provider: 'codex', preferredProvider: 'codex' });
  check('spend_down: a role whose CAPABILITY was already promoted is not promoted twice', promoted.result?.capability === 'frontier_max' && promoted.result?.effort === 'xhigh', JSON.stringify({ capability: promoted.result?.capability, effort: promoted.result?.effort }));
  check('spend_down: and the cap is explained', /one promotion per decision/.test(promoted.result?.effortStep?.why ?? ''), promoted.result?.effortStep?.why ?? '');

  const clamped = await ask('healthy-current', { role: 'judge', provider: 'codex', preferredProvider: 'codex' });
  check('spend_down: a step off the top of the ladder CLAMPS', clamped.result?.effort === 'xhigh', JSON.stringify(clamped.result?.effort));
  check('spend_down: and says the ladder ran out', /clamped at codex's ceiling/.test(clamped.result?.effortStep?.why ?? ''), clamped.result?.effortStep?.why ?? '');

  // 7. THE COMPOSED PATH: a cross-provider mode shift, and then a fallover walk
  //    running on top of it. Everything describing "who runs this" must name the
  //    FINAL landing rather than the provider the shift picked on the way.
  const composed = await ask('claude-overpace', { role: 'judge' });
  check('composed: the mode came from the conserving claude subject', composed.result?.mode === 'conserve' && composed.result?.capacity?.provider === 'claude', JSON.stringify({ mode: composed.result?.mode, subject: composed.result?.capacity?.provider }));
  check('composed: the mode shift moved the capability across providers', composed.result?.baseCapability === 'frontier_plus' && composed.result?.capability === 'cheap', JSON.stringify({ base: composed.result?.baseCapability, capability: composed.result?.capability }));
  check('composed: the walk then ran on top of the shift and landed elsewhere again', composed.result?.provider === 'copilot' && composed.result?.model === 'gpt-5-mini', JSON.stringify(composed.result));
  check('composed: fellOverFrom names the row the WALK passed over', composed.result?.fellOverFrom?.provider === 'codex', JSON.stringify(composed.result?.fellOverFrom));
  check('composed: shiftCapacity describes the FINAL landing, not the shift\'s', composed.result?.shiftCapacity?.provider === 'copilot', JSON.stringify(composed.result?.shiftCapacity?.provider));
  check('composed: the event names the landing provider and its own health', composed.event?.provider === 'copilot' && composed.event?.health === composed.result?.shiftCapacity?.health, JSON.stringify({ provider: composed.event?.provider, health: composed.event?.health }));
}

// runAvailabilityAssertions RESTORES WHAT IT BREAKS, so it no longer has to run
// last. It used to: an `unavailable` verdict was held for the full ten-minute
// catalogTTL, so once codex answered the catalog with an empty list nothing in
// the rest of the run could route to it again. That staleness was the bug (a
// provider somebody has just installed is exactly the one whose verdict changed),
// and now that a forced refresh re-probes an unavailable entry the scenario can
// flip the knob back and prove codex is routable again before it returns. The
// slice 2 cases that run after it are the second, independent proof: their
// fallovers land on codex.
async function runAvailabilityAssertions(caller, fakeURL) {
  console.log('\nlive provider availability:');
  await setScenario(fakeURL, 'healthy-current', true);

  // Before: codex is not flagged, the fake 404s its catalog, and "we could not
  // ask" must route exactly as it always did.
  const before = await caller.call('routing.select', {
    ...routingRequest('availability-before', ''),
    provider: '',
    preferredProvider: '',
    role: 'implementer',
  });
  check('availability: an UNKNOWN provider still gets the work (fail open)', before?.provider === 'codex', JSON.stringify({ provider: before?.provider, reason: reasonsOf(before) }));

  await setProviderAvailable(fakeURL, 'codex', false);
  const probesBefore = await modelProbeCount(fakeURL, 'codex');
  // The refresh is kicked BY a decision and runs in the background, so the ask
  // that flags the provider is not the ask that sees it — and the refresh is
  // rate-limited, so this poll has to keep ASKING rather than only watching.
  // That is the contract: no ambient probing, at most one refresh per decision.
  const reprobed = await waitFor(async () => {
    await caller.call('routing.select', { ...routingRequest('availability-kick', ''), provider: '', preferredProvider: '', role: 'implementer' });
    return (await modelProbeCount(fakeURL, 'codex')) > probesBefore;
  }, 20000);
  check('availability: a decision kicks a background re-probe of the provider catalog', !!reprobed, `codex model probes stayed at ${probesBefore}`);

  const after = await waitFor(async () => {
    const d = await caller.call('routing.select', {
      ...routingRequest('availability-after', ''),
      provider: '',
      preferredProvider: '',
      role: 'implementer',
    });
    return d?.provider === 'claude' ? d : null;
  }, 20000);
  check('availability: a provider that reports no launchable model is routed AROUND', !!after, 'the decision stayed on codex after its catalog went empty');
  if (after) {
    check('availability: the reason names the live probe, not the allowance', /is not available to launch right now/.test(reasonsOf(after)), reasonsOf(after));
    check('availability: the reason says what the probe SAW and does not claim a missing CLI', /ran and reported no launchable model/.test(reasonsOf(after)) && !/not installed/.test(reasonsOf(after)), reasonsOf(after));
    check('availability: and the allowance it skipped was healthy, so nothing else explains the move', after?.capacity?.health === 'green', JSON.stringify(after?.capacity?.health));
  }

  // AND IT COMES BACK. The verdict above is cached, and until the forced
  // refresh started re-probing unavailable entries it was cached for ten
  // minutes — so a provider that had just been installed stayed unroutable for
  // ten minutes after it started working. Flipping the knob back and asking
  // again is the whole assertion.
  await setProviderAvailable(fakeURL, 'codex', true);
  const restored = await waitFor(async () => {
    const d = await caller.call('routing.select', {
      ...routingRequest('availability-restored', ''),
      provider: '',
      preferredProvider: '',
      role: 'implementer',
    });
    return d?.provider === 'codex' ? d : null;
  }, 30000);
  check('availability: a provider that starts answering again is routable again inside the catalog TTL', !!restored, 'codex stayed unavailable after its catalog was restored — the forced refresh is not re-probing unavailable entries');
  if (restored) {
    check('availability: and the restored answer carries no fallover at all', !restored?.fellOverFrom, JSON.stringify(restored?.fellOverFrom));
  }
}

try {
  console.log('building hub...');
  execFileSync('go', ['build', '-o', hubBin, './cmd/hub'], { cwd: hubDir, stdio: 'inherit' });

  const fakePort = await freePort();
  const hubPort = await freePort();
  const fake = await startFake(fakePort);
  fakeChild = fake.child;
  seedFixtures();
  startHub(hubPort, fake.url);
  const caller = await connectWithRetry(hubPort, 'routing-limit');
  caller.subscribe('routing.*');

  console.log(`fake claudemon: ${fake.url}`);
  console.log(`scratch hub:     ws://127.0.0.1:${hubPort}/bus`);

  console.log('\nfake claudemon scenarios:');
  await setScenario(fake.url, 'healthy-current', true);
  let report = await fakeJSON(fake.url, '/usage/report');
  let five = codexFive(report);
  check('healthy-current: Codex 5h percentage is a real ok scalar', five?.used_percent?.state === 'ok' && five.used_percent.value === 12.0, JSON.stringify(five));
  check('healthy-current: Codex 5h reset is in the future', five?.resets_at > report.generated_at && five?.is_current === true, JSON.stringify(five));

  await setScenario(fake.url, 'stale-codex', true);
  report = await fakeJSON(fake.url, '/usage/report');
  five = codexFive(report);
  check('stale-codex: captured used_percent is verbatim 67.0', five?.used_percent?.state === 'ok' && five.used_percent.value === 67.0, JSON.stringify(five));
  check('stale-codex: captured resets_at is verbatim 1787953526', five?.resets_at === 1787953526, JSON.stringify(five));
  check('stale-codex: captured window_minutes is verbatim 300', five?.window_minutes === 300, JSON.stringify(five));
  check('stale-codex: fake marks the rolled window non-current', five?.is_current === false, JSON.stringify(five));

  await setScenario(fake.url, 'reset-now', true);
  report = await fakeJSON(fake.url, '/usage/report');
  five = codexFive(report);
  check('reset-now: resets_at equals generated_at exactly', five?.resets_at === report.generated_at, JSON.stringify({ generated_at: report.generated_at, five }));
  check('reset-now: equality is non-current', five?.is_current === false, JSON.stringify(five));

  await setScenario(fake.url, 'copilot-403', true);
  report = await fakeJSON(fake.url, '/usage/report');
  const copilot = provider(report, 'copilot');
  const copilotWindows = Object.values(copilot?.accounts?.[0]?.windows ?? {});
  check('copilot-403: Copilot account is present', !!copilot?.accounts?.[0], JSON.stringify(copilot));
  check(
    'copilot-403: every window is unavailable with a 403 reason',
    copilotWindows.length === 3 &&
      copilotWindows.every((w) => w.used_percent?.state === 'unavailable' && /403/.test(w.used_percent?.reason ?? '')),
    JSON.stringify(copilotWindows),
  );

  await setScenario(fake.url, 'absent-opencode-pi', true);
  report = await fakeJSON(fake.url, '/usage/report');
  const names = report.providers.map((p) => p.provider).sort();
  check('absent-opencode-pi: opencode is absent from the usage report', !names.includes('opencode'), JSON.stringify(names));
  check('absent-opencode-pi: pi is absent from the usage report', !names.includes('pi'), JSON.stringify(names));

  await clearFakeRequests(fake.url);
  await runRoutingCases(caller, fake.url);

  await runPacingAssertions(caller, fake.url);

  const decisionId = await runDecisionRecordAssertions(caller, fake.url);
  await runCeilingAwareSelectAssertions(caller);
  // AVAILABILITY RUNS BEFORE SLICE 2, not after. It used to have to go last
  // because the unavailable verdict it creates was sticky for catalogTTL; it now
  // restores codex before it returns, and the slice 2 fallovers that follow are
  // an independent proof that the restore took.
  await runAvailabilityAssertions(caller, fake.url);
  await runSlice2RoutingAssertions(caller, fake.url);
  await runSpawnBindingAssertions(hubPort, decisionId);

  caller.close();
  fake.child.kill('SIGTERM');
  fakeChild = null;

  if (pendingChecks.length > 0 && requireRouting) {
    failures += pendingChecks.length;
    console.log(`\n${pendingChecks.length} pending routing assertion(s) are fatal because ROUTING_HARNESS_REQUIRE_ROUTING=1`);
  }

  console.log('\nsummary:');
  console.log(`  active checks:  ${activeChecks}`);
  console.log(`  pending checks: ${pendingChecks.length}`);
  console.log(`  failures:       ${failures}`);
  if (pendingChecks.length > 0) {
    console.log('  parked routing assertions:');
    for (const p of pendingChecks) console.log(`    - ${p.name}: ${p.reason}`);
  }
  console.log(failures === 0 ? '\nACTIVE CHECKS PASSED' : '\nCHECKS FAILED');
} finally {
  try {
    hub?.kill('SIGKILL');
  } catch {}
  try {
    fakeChild?.kill('SIGKILL');
  } catch {}
  fs.rmSync(tmp, { recursive: true, force: true });
}

process.exit(failures === 0 ? 0 : 1);
