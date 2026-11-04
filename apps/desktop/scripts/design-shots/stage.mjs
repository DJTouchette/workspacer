/**
 * The stage: one REAL hub binary on an ephemeral port, every byte of state in a
 * scratch dir, serving the REAL web bundle at /app/ (and the embedded /m PWA),
 * with a fake capability provider attached over /bus that answers out of the
 * fabricated world in world.mjs.
 *
 * This is tests/e2e/fixtures/appHub.ts re-expressed as a plain .mjs (so it runs
 * with `node`, outside Playwright Test) and pointed at an invented world rather
 * than the e2e fixture's sessions. The isolation properties are the same:
 *
 *   - the hub gets HOME + every XDG_* redirected into the scratch dir, and every
 *     path flag pinned there too, so nothing can reach a real ~/.config;
 *   - the port is whatever the kernel hands out for :0, and the live stack's
 *     ports (7890/7891/7895) are refused outright;
 *   - no brain / claudemon is started — the whole capability surface is the
 *     provider below, so nothing dials a real daemon.
 */
import { spawn, spawnSync } from 'child_process';
import * as fs from 'fs';
import * as net from 'net';
import * as path from 'path';

export const DESKTOP_DIR = path.resolve(import.meta.dirname, '../..');
export const REPO = path.resolve(DESKTOP_DIR, '../..');
export const WEB_DIR = path.join(DESKTOP_DIR, 'dist/web');
export const HOST_TOKEN = 'design-shots-host-token';
const LIVE_PORTS = [7890, 7891, 7895, 7897];

const DESKTOP_METHODS = JSON.parse(
  fs.readFileSync(path.join(REPO, 'contracts/desktop-service-methods.json'), 'utf8'),
);

export function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const port = srv.address().port;
      srv.close(() => (LIVE_PORTS.includes(port) ? freePort().then(resolve, reject) : resolve(port)));
    });
  });
}

async function waitForHealth(url, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(url + '/health')).ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('hub did not become healthy at ' + url);
}

// ── builds ──────────────────────────────────────────────────────────────────

/** Newest mtime under `p` (skipping node_modules). */
function newestMtime(p) {
  let st;
  try {
    st = fs.statSync(p);
  } catch {
    return 0;
  }
  if (!st.isDirectory()) return st.mtimeMs;
  let max = 0;
  for (const e of fs.readdirSync(p)) {
    if (e === 'node_modules') continue;
    max = Math.max(max, newestMtime(path.join(p, e)));
  }
  return max;
}

/** Rebuild dist/web when any renderer source is newer than it (or `force`). */
export function ensureWebBundle({ force = false, skip = false, log = console.log } = {}) {
  const built = path.join(WEB_DIR, 'index.html');
  if (skip && fs.existsSync(built)) return;
  const builtAt = fs.existsSync(built) ? fs.statSync(built).mtimeMs : 0;
  const srcAt = Math.max(
    newestMtime(path.join(DESKTOP_DIR, 'src/renderer/src')),
    newestMtime(path.join(DESKTOP_DIR, 'src/renderer/index.html')),
    newestMtime(path.join(DESKTOP_DIR, 'src/renderer/vite.config.web.ts')),
    newestMtime(path.join(DESKTOP_DIR, 'src/main/shared')),
  );
  if (!force && builtAt && builtAt >= srcAt) {
    log('web bundle is current (dist/web)');
    return;
  }
  if (!fs.existsSync(path.join(DESKTOP_DIR, 'src/renderer/node_modules'))) {
    throw new Error(
      'renderer node_modules missing — run: (cd apps/desktop && npm install --ignore-scripts) ' +
        '&& (cd apps/desktop/src/renderer && npm install --ignore-scripts)',
    );
  }
  log('building web bundle (npm run build:renderer:web)…');
  const r = spawnSync('npm', ['run', 'build:renderer:web'], {
    cwd: DESKTOP_DIR,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  if (r.status !== 0) throw new Error('web bundle build failed:\n' + r.stderr + r.stdout);
}

/** Build (or reuse) the hub contract fixture: the real Rust hub's bus/HTTP
 *  adapters with no execution backends. Same binary the `app` e2e project uses. */
export function ensureHubBinary({ log = console.log } = {}) {
  const prebuilt = process.env.WKS_TEST_HUB_FIXTURE_BINARY;
  if (prebuilt) return prebuilt;
  log('building hub fixture (cargo build --example hub_contract_fixture; cached after first run)…');
  const r = spawnSync(
    'cargo',
    [
      'build',
      '--locked',
      '--manifest-path',
      path.join(REPO, 'services/hub-rs/Cargo.toml'),
      '--features',
      'test-support',
      '--example',
      'hub_contract_fixture',
      '--message-format=json',
    ],
    { cwd: REPO, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, stdio: ['ignore', 'pipe', 'inherit'] },
  );
  if (r.status !== 0) throw new Error('hub fixture build failed');
  for (const line of r.stdout.trim().split('\n').reverse()) {
    try {
      const item = JSON.parse(line);
      if (item.reason === 'compiler-artifact' && item.target?.name === 'hub_contract_fixture' && item.executable)
        return item.executable;
    } catch {
      /* not json */
    }
  }
  throw new Error('cargo did not report the hub_contract_fixture executable');
}

// ── config merge with the renderer's semantics ─────────────────────────────

export function deepMerge(base, patch) {
  const out = { ...base };
  for (const [k, v] of Object.entries(patch ?? {})) {
    const prev = out[k];
    out[k] =
      v && typeof v === 'object' && !Array.isArray(v) && prev && typeof prev === 'object' && !Array.isArray(prev)
        ? deepMerge(prev, v)
        : v;
  }
  return out;
}

// ── the stage ───────────────────────────────────────────────────────────────

/**
 * @param {object} o
 * @param {string} o.stateRoot   scratch root; a fresh subdir is made per run
 * @param {object} o.world       from world.mjs: { config, layout, snapshots, handlers }
 */
export async function startStage({ stateRoot, world, hubBin, log = console.log }) {
  fs.mkdirSync(stateRoot, { recursive: true });
  const dir = fs.mkdtempSync(path.join(stateRoot, 'hub-'));
  for (const sub of ['home', 'config/workspacer', 'config/workspacer-hub', 'data', 'state', 'cache'])
    fs.mkdirSync(path.join(dir, sub), { recursive: true });
  const env = {
    ...process.env,
    HOME: path.join(dir, 'home'),
    USERPROFILE: path.join(dir, 'home'),
    XDG_CONFIG_HOME: path.join(dir, 'config'),
    XDG_DATA_HOME: path.join(dir, 'data'),
    XDG_STATE_HOME: path.join(dir, 'state'),
    XDG_CACHE_HOME: path.join(dir, 'cache'),
    CLAUDEMON_DB_PATH: path.join(dir, 'data', 'claudemon', 'state.db'),
  };
  const p = (...s) => path.join(dir, ...s);
  fs.writeFileSync(
    p('config/workspacer/tokens.json'),
    JSON.stringify([
      // A triage token so /m can be shot under a phone-shaped scope too.
      { token: 'design-shots-phone', scope: 'operator', label: 'Pixel 9', created: new Date().toISOString() },
    ]),
  );
  // layout.get/layout.set are HUB-LOCAL, so a seeded layout is planted in the
  // file the hub reads at startup.
  fs.writeFileSync(p('config/workspacer-hub/layout.json'), JSON.stringify({ version: 1, data: world.layout }));
  fs.writeFileSync(p('config/workspacer-hub/nodes.json'), JSON.stringify(world.nodes ?? []), { mode: 0o600 });
  if (world.jobs) fs.writeFileSync(p('config/workspacer-hub/jobs.json'), JSON.stringify(world.jobs));
  if (world.jobsHistory)
    fs.writeFileSync(p('config/workspacer-hub/jobs-history.json'), JSON.stringify(world.jobsHistory));

  const port = await freePort();
  const url = `http://127.0.0.1:${port}`;
  const proc = spawn(
    hubBin,
    [
      '--mode', 'browser',
      '--root', dir,
      '--listen', `127.0.0.1:${port}`,
      '--token', HOST_TOKEN,
      '--webapp-dir', WEB_DIR,
      '--tokens-file', p('config/workspacer/tokens.json'),
      '--layout-file', p('config/workspacer-hub/layout.json'),
      '--push-dir', p('config/workspacer-hub/push'),
      '--peers-file', p('config/workspacer/peers.json'),
      '--jobs-file', p('config/workspacer-hub/jobs.json'),
      '--nodes-file', p('config/workspacer-hub/nodes.json'),
    ],
    // stdin must stay OPEN: the hub's parentwatch treats EOF as "parent died".
    { stdio: ['pipe', 'pipe', 'pipe'], env },
  );
  const hubLog = fs.createWriteStream(p('hub.log'));
  proc.stdout.pipe(hubLog);
  proc.stderr.pipe(hubLog);
  await waitForHealth(url);

  // ── the fake provider ────────────────────────────────────────────────────
  let current = world;
  let liveConfig = structuredClone(world.config);
  const snapshots = new Map(world.snapshots.map((s) => [s.sessionId, s]));
  const calls = [];
  let outSeq = 0;
  const unhandled = new Map();
  const ws = new WebSocket(`ws://127.0.0.1:${port}/bus?token=${HOST_TOKEN}`);
  await new Promise((resolve, reject) => {
    ws.addEventListener('open', () => resolve());
    ws.addEventListener('error', () => reject(new Error('provider socket failed')));
  });
  const methods = [
    ...DESKTOP_METHODS.ownerMethods,
    ...(DESKTOP_METHODS.assetMethods ?? []),
    ...Object.keys(world.handlers),
    'config.get', 'config.getPath', 'config.reload', 'config.save',
    'sessions.snapshots', 'sessions.snapshot',
  ];
  ws.send(JSON.stringify({ op: 'register', methods: [...new Set(methods)] }));
  const reply = (id, result) => ws.send(JSON.stringify({ op: 'result', id, result }));
  const fail = (id, error) => ws.send(JSON.stringify({ op: 'error', id, error }));
  const publish = (type, data) =>
    ws.send(JSON.stringify({ op: 'publish', event: { type, source: 'design-shots', data } }));

  const ctx = {
    get config() {
      return liveConfig;
    },
    snapshots,
    publish,
    dir,
  };

  ws.addEventListener('message', async (ev) => {
    let f;
    try {
      f = JSON.parse(String(ev.data));
    } catch {
      return;
    }
    if (f.op !== 'call') return;
    const params = f.params ?? {};
    calls.push({ method: f.method, params });
    try {
      switch (f.method) {
        case 'config.get':
        case 'config.reload':
          return reply(f.id, liveConfig);
        case 'config.save':
          liveConfig = deepMerge(liveConfig, params);
          return reply(f.id, liveConfig);
        case 'desktop.saveConfig':
          liveConfig = deepMerge(liveConfig, params.partial ?? {});
          return reply(f.id, liveConfig);
        case 'config.getPath':
          return reply(f.id, '/home/dev/.config/workspacer/config.yaml');
        case 'sessions.snapshots':
          return reply(f.id, [...snapshots.values()]);
        case 'sessions.snapshot':
          return reply(f.id, snapshots.get(params.sessionId) ?? null);
      }
      const h = current.handlers[f.method];
      if (h) {
        const out = await h(params, ctx);
        if (out instanceof Error) return fail(f.id, out.message);
        return reply(f.id, out === undefined ? { ok: true } : out);
      }
      unhandled.set(f.method, (unhandled.get(f.method) ?? 0) + 1);
      return reply(f.id, { ok: true });
    } catch (err) {
      return fail(f.id, err instanceof Error ? err.message : String(err));
    }
  });

  log(`hub up at ${url}  (state: ${dir})`);
  return {
    url,
    appUrl: `${url}/app/?token=${HOST_TOKEN}`,
    mobileUrl: `${url}/m?token=${HOST_TOKEN}`,
    dir,
    calls,
    unhandled,
    publish,
    startedAt: Date.now(),
    /** Replace the hub's layout document (versioned + broadcast by the hub). */
    setLayout(layout) {
      ws.send(JSON.stringify({ op: 'call', id: `stage-${outSeq++}`, method: 'layout.set', params: { data: layout } }));
    },
    /** Back to the pristine world: layout, snapshots, config. */
    reset(next = current) {
      current = next;
      ws.send(JSON.stringify({ op: 'call', id: `stage-${outSeq++}`, method: 'layout.set', params: { data: next.layout } }));
      snapshots.clear();
      for (const s of next.snapshots) snapshots.set(s.sessionId, s);
      liveConfig = structuredClone(next.config);
      calls.length = 0;
    },
    pushSnapshot(s) {
      snapshots.set(s.sessionId, s);
      publish('agent.snapshot', s);
    },
    async stop({ keep = false } = {}) {
      try {
        ws.close();
      } catch {
        /* gone */
      }
      proc.kill('SIGKILL');
      await new Promise((r) => setTimeout(r, 150));
      if (!keep) fs.rmSync(dir, { recursive: true, force: true });
    },
  };
}
