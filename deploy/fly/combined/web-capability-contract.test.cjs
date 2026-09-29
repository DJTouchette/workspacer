'use strict';
const assert = require('node:assert/strict');
const test = require('node:test');
const { validateWebCapabilityInventory: validate } = require('./web-capability-contract.cjs');

function healthy() {
  return {
    registeredMethods: [
      'git.stage', 'git.unstage', 'git.commit', 'git.push', 'git.commitDiff',
      'git.commitNumstat', 'fs.watch', 'fs.unwatch', 'fleetWorkflows.request',
      'desktop.managerReplacement', 'desktop.filePickerList', 'desktop.readFileBytes',
      'desktop.sessionGrantReconcile', 'files.receiveUpload', 'agents.spawn',
      'plugins.manifests', 'federation.peersConfig', 'remote.tailscaleServe', 'ui.asset',
    ],
    launchReady: true,
    probes: { 'providers.checkAll': { status: 'ok' }, 'sessions.snapshots': { status: 'ok' } },
    idle: { mode: 'stop', timeoutSeconds: 900 },
    http: { pluginsCount: 2, examplesCount: 2 },
    network: { available: true, serveActive: true, canServe: true },
    runtime: { hub: 'ready', claudemon: 'ready', facade: 'ready' },
  };
}

test('owned backend passes without the retired provider callback', () => {
  const inventory = healthy();
  assert(!inventory.registeredMethods.includes('plugins.prepareLaunch'));
  assert.deepEqual(validate(inventory), { ok: true, failures: [] });
});

test('legacy callback or advertised spawn cannot substitute for launch readiness', () => {
  for (const readiness of [undefined, false, 'true', 1]) {
    const inventory = healthy();
    inventory.launchReady = readiness;
    inventory.registeredMethods.push('plugins.prepareLaunch');
    assert(validate(inventory).failures.includes('owned launch service unavailable'));
    assert.equal(validate(inventory).ok, false);
  }
});

test('readiness alone cannot hide missing public spawn or plugin service', () => {
  for (const method of ['agents.spawn', 'plugins.manifests']) {
    const inventory = healthy();
    inventory.registeredMethods = inventory.registeredMethods.filter(value => value !== method);
    assert(validate(inventory).failures.includes('missing ' + method));
  }
  const malformed = healthy();
  malformed.registeredMethods = malformed.registeredMethods.join(',');
  assert(validate(malformed).failures.includes('missing agents.spawn'));
});

test('public capability, daemon, probe, plugin and deployment checks remain effective', () => {
  const cases = [
    [value => { value.registeredMethods = value.registeredMethods.filter(m => m !== 'fs.watch'); }, 'missing fs.watch'],
    [value => { value.runtime.facade = 'starting'; }, 'runtime not ready'],
    [value => { value.probes['providers.checkAll'].status = 'error'; }, 'providers.checkAll: error'],
    [value => { delete value.http.pluginsCount; }, 'bundled plugins unavailable'],
    [value => { value.network.serveActive = false; }, 'network adapter unavailable'],
    [value => { value.idle.mode = 'observe'; }, 'idle stop policy changed'],
  ];
  for (const [mutate, failure] of cases) {
    const inventory = healthy(); mutate(inventory);
    const result = validate(inventory);
    assert.equal(result.ok, false); assert(result.failures.includes(failure), failure);
  }
});

test('Python remote wrapper collects and enforces actual health launch readiness', async () => {
  const { execFileSync } = require('node:child_process');
  const path = require('node:path');
  const vm = require('node:vm');
  // Capture the real remote program while replacing subprocess.run, so this
  // test cannot call flyctl or a live backend.
  const remoteCode = execFileSync('python3', ['-c', `
import runpy, shlex, subprocess, sys
script = sys.argv[1]
sys.argv = [script, 'fixture-app', 'fixture-machine', '--expect-parity']
subprocess.run = lambda argv, **kwargs: print(shlex.split(argv[-1])[2])
runpy.run_path(script, run_name='__main__')
`, path.join(__dirname, 'verify-web-capabilities.py')], { encoding: 'utf8' });
  execFileSync(process.execPath, ['--check'], { input: remoteCode });
  for (const ready of [true, false, undefined]) {
    const inventory = healthy();
    inventory.launchReady = ready;
    const timers = new Set();
    let finish, fail;
    const completed = new Promise((resolve, reject) => { finish = resolve; fail = reject; });
    const processState = { exitCode: 0 };
    const watchdog = setTimeout(() => fail(new Error('remote fixture did not finish')), 3000);
    class Socket {
      constructor() { queueMicrotask(() => this.onopen()); }
      close() {}
      send(raw) {
        const frame = JSON.parse(raw);
        const result = frame.method === 'machine.power'
          ? { idleMode: 'stop', idleTimeoutSeconds: 900 }
          : frame.method === 'desktop.agentRuntimeStatus' ? inventory.runtime
          : frame.method === 'remote.tailscaleInfo' ? inventory.network : {};
        queueMicrotask(() => this.onmessage({ data: JSON.stringify({ op: 'result', id: frame.id, result }) }));
      }
    }
    try {
      vm.runInNewContext(remoteCode, {
        module: { exports: {} },
        require(name) {
          assert.equal(name, 'node:fs');
          return { readFileSync() { return 'fixture-only-credential'; } };
        },
        WebSocket: Socket,
        AbortSignal,
        process: processState,
        console: { log(value) { finish(JSON.parse(value)); }, error(value) { fail(new Error(value)); } },
        setTimeout(callback, delay) { const timer = setTimeout(callback, delay); timers.add(timer); return timer; },
        clearTimeout,
        async fetch(url) {
          const route = new URL(url).pathname;
          return { ok: true, status: 200, async json() {
            if (route === '/health') return { methodNames: inventory.registeredMethods, methods: inventory.registeredMethods.length, launchReady: ready };
            return [{}, {}];
          } };
        },
      });
      const result = await completed;
      assert.equal(result.validation.ok, ready === true);
      assert.equal(processState.exitCode, ready === true ? 0 : 1);
      if (ready !== true) assert(result.validation.failures.includes('owned launch service unavailable'));
    } finally {
      clearTimeout(watchdog);
      for (const timer of timers) clearTimeout(timer);
    }
  }
});
