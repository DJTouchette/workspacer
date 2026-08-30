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
      '-tokens-file', '',
    ],
    { env, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  hub.stderr.on('data', (d) => process.env.HARNESS_VERBOSE && process.stderr.write(d));
  hub.stdout.on('data', (d) => process.env.HARNESS_VERBOSE && process.stdout.write(d));
}

function connect(hubPort, name) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${hubPort}/bus?token=${TOKEN}`);
    let seq = 0;
    const pendingCalls = new Map();
    const events = [];
    ws.onopen = () =>
      resolve({
        ws,
        events,
        subscribe(...topics) {
          ws.send(JSON.stringify({ op: 'subscribe', topics }));
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
      if (f.op !== 'result' && f.op !== 'error') return;
      const p = pendingCalls.get(f.id);
      if (!p) return;
      pendingCalls.delete(f.id);
      f.op === 'result' ? p.res(f.result) : p.rej(new Error(f.error));
    };
  });
}

async function connectWithRetry(hubPort, name, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try {
      return await connect(hubPort, name);
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

try {
  console.log('building hub...');
  execFileSync('go', ['build', '-o', hubBin, './cmd/hub'], { cwd: hubDir, stdio: 'inherit' });

  const fakePort = await freePort();
  const hubPort = await freePort();
  const fake = await startFake(fakePort);
  fakeChild = fake.child;
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
