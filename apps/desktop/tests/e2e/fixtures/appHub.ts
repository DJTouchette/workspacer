/**
 * Test rig for `/app` — the FULL React renderer running in a browser against
 * the hub, which is the surface the user wants as their primary interface.
 *
 * It is `mobileHub.ts`'s shape, widened. Same proven safety properties: the
 * REAL hub binary, an ephemeral `freePort()`, a scratch state dir, and a fake
 * capability provider attached over `/bus` with the host token. The client
 * under test therefore speaks the genuine wire protocol to the genuine router;
 * only the far side of the capability boundary is fabricated.
 *
 * Three things differ from the mobile rig, and each is deliberate:
 *
 *   1. **It serves the web bundle.** `--webapp-dir dist/web` makes the hub host
 *      `/app/` (`cmd/hub/main.go:733-750`), and the fixture rebuilds that
 *      bundle first — a stale bundle would mean the suite tests nothing. The
 *      rebuild is ~5s warm.
 *   2. **A much wider method surface.** `/m` calls ~20 bus methods; `/app`
 *      calls ~70 (see the audit at
 *      `.workspacer/reports/2026-08-24-web-client-completeness.md`). Rather
 *      than enumerate them and have the list rot, the provider registers the
 *      known surface and answers anything unrecognised with a benign default,
 *      while RECORDING it — so `hub.calls` stays a faithful log.
 *   3. **Every state path is scratch, asserted.** See `scratchState.ts`. The
 *      mobile rig passes `--tokens-file/--layout-file/--push-dir` but leaves
 *      `--peers-file` and `--jobs-file` on their real defaults; here HOME and
 *      the XDG variables are redirected too, so a flag we forget cannot reach
 *      the developer's `~/.config/workspacer`.
 */
import { spawn, spawnSync, type ChildProcess } from 'child_process';
import * as fs from 'fs';
import * as http from 'http';
import * as path from 'path';
import {
  assertNoLiveStateHandles,
  withBuildLock,
  assertScratchEnv,
  assertScratchPath,
  freePort,
  makeScratchDir,
  removeScratchDir,
  scratchEnv,
} from './scratchState';
import { FIXTURE_LIBRARY, FIXTURE_PROVIDERS, FIXTURE_RECENT, FIXTURE_SESSIONS } from './mobileHub';
import { CONFIG_DEFAULTS } from '../../../src/renderer/src/hooks/configDefaults.generated';

/**
 * `config.get` must answer a WHOLE config, not the sparse object the mobile rig
 * uses: `/m` reads two keys, but the desktop renderer reads `ui.theme` during
 * its first render and throws through the error boundary if it is absent. So we
 * answer the real generated defaults (the same file the app and the brain share)
 * with the fixture's directories layered on top.
 */
export const APP_CONFIG = {
  ...CONFIG_DEFAULTS,
  // Otherwise the first-run welcome overlay covers the whole workspace and no
  // spec can click anything behind it (App.tsx:592).
  onboardingDismissed: true,
  directories: {
    favourites: [
      '/home/djtouchette/Work/worky/workspacer',
      '/home/djtouchette/Work/rivet-umbrella/rivet',
    ],
    recent: ['/home/djtouchette/Work/rivet-umbrella/recon', '/home/djtouchette'],
  },
} as Record<string, any>;

/** Rendered by the fixture's content server; the string a pane must actually
 *  paint for "it loaded" to be more than "the element exists". */
const CONTENT_MARKER = 'PANE-CONTENT-LOADED';

const REPO = path.resolve(__dirname, '../../../../..');
const HUB_DIR = path.join(REPO, 'services/hub');
const HUB_BIN = path.join(HUB_DIR, 'hub');
const DESKTOP_DIR = path.join(REPO, 'apps/desktop');
const WEB_DIR = path.join(DESKTOP_DIR, 'dist/web');

export const HOST_TOKEN = 'app-e2e-host-token';

export interface CallRecord {
  method: string;
  params: any;
}

/** A per-test override for one bus method. Return value is sent as the result;
 *  throw to make the call fail the way a missing provider would. */
export type Handler = (params: any) => unknown;

/** The slice of app state the hub's layout document carries. */
export interface SharedLayout {
  agents: any[];
  activeAgentId: string;
}

export interface AppHubOptions {
  /** Seed the hub's layout document. `/app` adopts it during hydration
   *  (`useLayoutSync.ts:167`, App passes adoptSharedLayout: true), which is how
   *  a spec gets straight to a workspace bound to a fixture session instead of
   *  driving the spawn dialog first. */
  layout?: SharedLayout;
}

export interface AppHub {
  /** http://127.0.0.1:<ephemeral> */
  url: string;
  /** The URL a browser opens: /app/ with the token in the query string. */
  appUrl: string;
  /** pid of the hub process — for assertNoLiveStateHandles. */
  pid: number;
  /** The scratch directory holding every byte of this hub's state. */
  scratchDir: string;
  /** A tiny static site on its own ephemeral port, for panes that load a URL
   *  (plugin panes, the Browser pane). Serving real content is the only way to
   *  tell "rendered the page" from "rendered a blank box". */
  contentUrl: string;
  /** The marker text `contentUrl` renders — assert on this, not on the URL. */
  contentMarker: string;
  /** Replace the layout document and broadcast layout.changed. */
  setLayout(layout: SharedLayout | null): void;
  /** Every call the client made, in order. */
  calls: CallRecord[];
  callsTo(method: string): CallRecord[];
  /** Wait until the client has called `method`, or throw. */
  waitForCall(method: string, timeoutMs?: number): Promise<CallRecord>;
  /** Override one method for the current test. Cleared by reset(). */
  stub(method: string, handler: Handler): void;
  /** Replace a session snapshot and broadcast it as an agent.snapshot event. */
  pushSnapshot(snap: any): void;
  /** Publish an arbitrary bus event (pty.exit, layout.changed, …). */
  publish(type: string, data: unknown): void;
  snapshots: Map<string, any>;
  /** Restore the pristine fleet, drop per-test stubs, clear the call log. */
  reset(): void;
  stop(): Promise<void>;
}

/** Config merge with the renderer's semantics: objects merge, arrays and
 *  scalars replace wholesale (see lib/configPatch). */
function deepMerge(base: Record<string, any>, patch: Record<string, any>): Record<string, any> {
  const out: Record<string, any> = { ...base };
  for (const [k, v] of Object.entries(patch)) {
    const prev = out[k];
    out[k] =
      v &&
      typeof v === 'object' &&
      !Array.isArray(v) &&
      prev &&
      typeof prev === 'object' &&
      !Array.isArray(prev)
        ? deepMerge(prev, v)
        : v;
  }
  return out;
}

async function waitForHealth(url: string, timeoutMs = 20000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const r = await fetch(url + '/health');
      if (r.ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('hub did not become healthy at ' + url);
}

/**
 * Make sure the hub binary and the web bundle are current.
 *
 * Both are load-bearing — the hub is what serves `/app`, and `dist/web` IS the
 * thing under test, so a stale bundle would mean the suite tests nothing. But
 * rebuilding unconditionally costs ~5s on every spec file, and Playwright runs
 * spec files in parallel workers that would race each other writing the same
 * outputs. So:
 *
 *   - `withBuildLock` serialises it across worker processes, and
 *   - the bundle is only rebuilt when a renderer source file is newer than it.
 *
 * `WKS_E2E_SKIP_WEB_BUILD=1` skips the vite step entirely for a fast iteration
 * loop on the specs themselves.
 */
function build(): void {
  withBuildLock(() => {
    const hub = spawnSync('go', ['build', '-o', 'hub', './cmd/hub'], {
      cwd: HUB_DIR,
      encoding: 'utf8',
    });
    if (hub.status !== 0) throw new Error('failed to build hub: ' + hub.stderr);

    if (process.env.WKS_E2E_SKIP_WEB_BUILD !== '1' && webBundleIsStale()) {
      const web = spawnSync('npm', ['run', 'build:renderer:web'], {
        cwd: DESKTOP_DIR,
        encoding: 'utf8',
      });
      if (web.status !== 0) {
        throw new Error('failed to build the web bundle: ' + (web.stderr || web.stdout));
      }
    }
  });

  if (!fs.existsSync(path.join(WEB_DIR, 'index.html'))) {
    throw new Error(
      `no web bundle at ${WEB_DIR} — the hub would serve nothing at /app. ` +
        `Run: npm run build:renderer:web`,
    );
  }
}

/** True when any renderer source file is newer than the built bundle (or there
 *  is no bundle). Walking the source tree costs ~50ms; a needless vite build
 *  costs 5s on every spec file. */
function webBundleIsStale(): boolean {
  const built = path.join(WEB_DIR, 'index.html');
  let builtAt: number;
  try {
    builtAt = fs.statSync(built).mtimeMs;
  } catch {
    return true;
  }
  const roots = [
    path.join(DESKTOP_DIR, 'src/renderer/src'),
    path.join(DESKTOP_DIR, 'src/renderer/index.html'),
    path.join(DESKTOP_DIR, 'src/renderer/vite.config.web.ts'),
    path.join(DESKTOP_DIR, 'src/main/shared'),
  ];
  const newerThanBuild = (p: string): boolean => {
    let st: fs.Stats;
    try {
      st = fs.statSync(p);
    } catch {
      return false;
    }
    if (!st.isDirectory()) return st.mtimeMs > builtAt;
    for (const entry of fs.readdirSync(p)) {
      if (entry === 'node_modules') continue;
      if (newerThanBuild(path.join(p, entry))) return true;
    }
    return false;
  };
  return roots.some(newerThanBuild);
}

export async function startAppHub(opts: AppHubOptions = {}): Promise<AppHub> {
  build();

  const dir = makeScratchDir('wks-app-e2e');
  const env = scratchEnv(dir);
  assertScratchEnv(env);

  const scratch = (...p: string[]) => assertScratchPath(path.join(dir, ...p), p.join('/'));
  const tokensFile = scratch('config', 'workspacer', 'tokens.json');
  fs.mkdirSync(path.dirname(tokensFile), { recursive: true });
  fs.writeFileSync(tokensFile, '[]');

  // The layout document is HUB-LOCAL: `layout.get`/`layout.set` are answered by
  // the hub's own layout service (internal/layout), never routed to a bus
  // provider — so a seeded layout has to be planted in the file the hub reads
  // at startup, not answered from the fake provider.
  const layoutFile = scratch('config', 'workspacer-hub', 'layout.json');
  fs.mkdirSync(path.dirname(layoutFile), { recursive: true });
  if (opts.layout) {
    fs.writeFileSync(layoutFile, JSON.stringify({ version: 1, data: opts.layout }));
  }

  // A stand-in for whatever a pane would load in a webview: a plugin sidecar,
  // a widget board, a site in the Browser pane. Same-origin isn't assumed —
  // Playwright reads across frame origins, and the real thing is cross-origin
  // too (a plugin sidecar has its own port).
  const contentPort = await freePort();
  const contentServer = http.createServer((_req, res) => {
    res.writeHead(200, { 'Content-Type': 'text/html' });
    res.end(
      `<!doctype html><meta charset=utf-8><title>pane content</title><body><h1>${CONTENT_MARKER}</h1>`,
    );
  });
  await new Promise<void>((r) => contentServer.listen(contentPort, '127.0.0.1', () => r()));

  const port = await freePort();
  const url = `http://127.0.0.1:${port}`;
  const proc: ChildProcess = spawn(
    HUB_BIN,
    [
      '--addr',
      `127.0.0.1:${port}`,
      '--token',
      HOST_TOKEN,
      '--webapp-dir',
      WEB_DIR,
      // Every path flag pinned into the scratch tree. The env redirect above
      // would already do this via the flags' defaults; passing them too means
      // a change to either mechanism alone cannot leak.
      '--tokens-file',
      tokensFile,
      '--layout-file',
      layoutFile,
      '--push-dir',
      scratch('config', 'workspacer-hub', 'push'),
      '--peers-file',
      scratch('config', 'workspacer', 'peers.json'),
      '--jobs-file',
      scratch('config', 'workspacer-hub', 'jobs.json'),
      // No brain, so no claudemon: the whole capability surface is the fake
      // provider below, and nothing can dial the developer's :7891.
      '--brain-scope',
      'off',
      '--plugins-dir',
      '',
    ],
    // stdin must stay OPEN: the hub's parentwatch treats a closed stdin as "my
    // parent died" and shuts down immediately.
    { stdio: ['pipe', 'pipe', 'pipe'], env },
  );
  proc.stderr?.on('data', (b) => {
    const s = String(b);
    if (/panic|fatal/i.test(s)) console.error('[hub]', s.trim());
  });

  await waitForHealth(url);
  // Layer 3 of the blast shield: the hub is up and has opened its files. Prove
  // it holds nothing of the developer's. Throws loudly if it does.
  assertNoLiveStateHandles(proc.pid!);

  const snapshots = new Map<string, any>(FIXTURE_SESSIONS.map((s) => [s.sessionId, s]));
  const calls: CallRecord[] = [];
  const stubs = new Map<string, Handler>();
  // The live config this hub answers with. `config.save` MUST merge and return
  // the WHOLE document: ConfigContext assigns the save result straight into
  // state (`ConfigContext.tsx:88`), so a handler that returns `{ok:true}` blanks
  // `config.ui` and the app dies in useTheme on the next render. The renderer
  // saves during boot (keybinding-preset migration), so this is on the critical
  // path, not an edge case.
  let liveConfig: Record<string, any> = structuredClone(APP_CONFIG);
  const callWaiters: Array<{ method: string; resolve: (c: CallRecord) => void }> = [];
  let outSeq = 0;

  const ws = new WebSocket(`ws://127.0.0.1:${port}/bus?token=${HOST_TOKEN}`);
  await new Promise<void>((resolve, reject) => {
    ws.addEventListener('open', () => resolve());
    ws.addEventListener('error', () => reject(new Error('provider socket failed')));
  });
  ws.send(JSON.stringify({ op: 'register', methods: METHODS }));

  const reply = (id: string, result: unknown) =>
    ws.send(JSON.stringify({ op: 'result', id, result }));
  const fail = (id: string, message: string) =>
    ws.send(JSON.stringify({ op: 'error', id, error: message }));

  ws.addEventListener('message', (ev: MessageEvent) => {
    let f: any;
    try {
      f = JSON.parse(String(ev.data));
    } catch {
      return;
    }
    if (f.op !== 'call') return;
    const params = f.params ?? {};
    const rec: CallRecord = { method: f.method, params };
    calls.push(rec);
    for (let i = callWaiters.length - 1; i >= 0; i--) {
      if (callWaiters[i].method === f.method) callWaiters.splice(i, 1)[0].resolve(rec);
    }

    // These are ROUTED (so the call reaches us and is recorded) but answered
    // with an error, so the refusing case is what a spec gets unless it stubs
    // otherwise. See HEADLESS_GAP_METHODS for why that is the useful default.
    if (HEADLESS_GAP_METHODS.includes(f.method as never) && !stubs.has(f.method)) {
      return fail(f.id, `no provider for ${f.method}`);
    }

    const stub = stubs.get(f.method);
    if (stub) {
      try {
        return reply(f.id, stub(params));
      } catch (err) {
        // A thrown stub reproduces "no provider registered this method" — the
        // exact failure mode the web client hits for a desktop-only capability.
        return fail(f.id, err instanceof Error ? err.message : String(err));
      }
    }

    switch (f.method) {
      case 'sessions.snapshots':
        return reply(f.id, [...snapshots.values()]);
      case 'sessions.snapshot':
        return reply(f.id, snapshots.get(params.sessionId) ?? null);
      case 'sessions.conversation':
        return reply(f.id, { seq: 1, items: [] });
      case 'sessions.recent':
        return reply(f.id, FIXTURE_RECENT);
      case 'sessions.list':
        return reply(f.id, []);
      case 'config.get':
      case 'config.reload':
        return reply(f.id, liveConfig);
      case 'config.save':
        liveConfig = deepMerge(liveConfig, params ?? {});
        return reply(f.id, liveConfig);
      case 'config.getPath':
        return reply(f.id, path.join(dir, 'config', 'workspacer', 'config.yaml'));
      case 'library.list':
        return reply(f.id, FIXTURE_LIBRARY);
      case 'providers.checkAll':
        return reply(f.id, FIXTURE_PROVIDERS);
      case 'providers.listModels':
        return reply(f.id, [{ id: 'gpt-5.4', label: 'gpt-5.4', default: true }]);
      case 'claude.listModels':
        return reply(f.id, {
          defaultModel: 'opus',
          aliases: [
            { value: 'claude-opus-5[1m]', label: 'Opus 5 (1M)' },
            { value: 'claude-sonnet-5', label: 'Sonnet 5' },
          ],
          seen: ['claude-fable-5'],
        });
      case 'claude.sessionsForDir':
        return reply(f.id, []);
      case 'agents.spawn':
        return reply(f.id, {
          sessionId: 'spawned-1',
          ...(typeof params.message === 'string' && params.message.trim()
            ? { messageQueued: true }
            : {}),
        });
      case 'layouts.list':
        return reply(f.id, []);
      case 'federation.peers':
        return reply(f.id, []);
      case 'app.getCwd':
        return reply(f.id, APP_CONFIG.directories.favourites[0]);
      case 'app.supervisorHome':
        return reply(f.id, APP_CONFIG.directories.favourites[0]);
      case 'fs.listDir':
      case 'fs.listEntries':
        return reply(f.id, []);
      case 'git.status':
        return reply(f.id, { branch: 'master', staged: [], unstaged: [], untracked: [] });
      case 'analytics.summary':
        return reply(f.id, {});
      case 'analytics.recent':
        return reply(f.id, []);
      case 'jobs.list':
        return reply(f.id, []);
      case 'plugins.tools':
        return reply(f.id, []);
      default:
        // Deliberately benign. The call is still recorded, so a test that cares
        // asserts on `calls`; a test that does not is not derailed by a method
        // nobody has modelled yet.
        return reply(f.id, { ok: true });
    }
  });

  const publish = (type: string, data: unknown) =>
    ws.send(JSON.stringify({ op: 'publish', event: { type, source: 'app-e2e', data } }));

  return {
    url,
    contentUrl: `http://127.0.0.1:${contentPort}/`,
    contentMarker: CONTENT_MARKER,
    appUrl: `${url}/app/?token=${HOST_TOKEN}`,
    pid: proc.pid!,
    scratchDir: dir,
    calls,
    callsTo: (m) => calls.filter((c) => c.method === m),
    async waitForCall(method, timeoutMs = 10000) {
      const already = calls.find((c) => c.method === method);
      if (already) return already;
      return new Promise<CallRecord>((resolve, reject) => {
        const t = setTimeout(
          () => reject(new Error(`no ${method} call within ${timeoutMs}ms`)),
          timeoutMs,
        );
        callWaiters.push({
          method,
          resolve: (c) => {
            clearTimeout(t);
            resolve(c);
          },
        });
      });
    },
    stub: (method, handler) => void stubs.set(method, handler),
    setLayout(layout) {
      // Goes through the hub's own layout service (host token = trusted), which
      // versions it and broadcasts layout.changed to every connected client —
      // the same path another desktop/web client would take.
      ws.send(
        JSON.stringify({
          op: 'call',
          id: `fixture-${outSeq++}`,
          method: 'layout.set',
          params: { data: layout },
        }),
      );
    },
    pushSnapshot(snap) {
      snapshots.set(snap.sessionId, snap);
      publish('agent.snapshot', snap);
    },
    publish,
    snapshots,
    reset() {
      // The layout document is owned by the HUB, not by this fixture's state,
      // so a test that swapped it (setLayout) would otherwise leave every later
      // test in someone else's workspace. Put the seed back the same way a
      // client would.
      if (opts.layout) {
        ws.send(
          JSON.stringify({
            op: 'call',
            id: `fixture-${outSeq++}`,
            method: 'layout.set',
            params: { data: opts.layout },
          }),
        );
      }
      snapshots.clear();
      for (const s of FIXTURE_SESSIONS) snapshots.set(s.sessionId, s);
      calls.length = 0;
      stubs.clear();
      callWaiters.length = 0;
      liveConfig = structuredClone(APP_CONFIG);
    },
    async stop() {
      try {
        ws.close();
      } catch {
        /* already gone */
      }
      proc.kill('SIGKILL');
      await new Promise<void>((r) => contentServer.close(() => r()));
      await new Promise((r) => setTimeout(r, 100));
      removeScratchDir(dir);
    },
  };
}

/**
 * The bus methods the provider claims. Derived from the `client.call(...)` sites
 * in `src/renderer/src/backend/webBackend.ts`; anything not here still reaches
 * the default arm above (the hub routes unregistered methods to nobody, so a
 * method missing from this list REJECTS — which is exactly what a real headless
 * gap does, and some tests want that).
 */
const METHODS = [
  'agents.sendMessage',
  'agents.spawn',
  'analytics.recent',
  'analytics.summary',
  'app.getCwd',
  'app.supervisorHome',
  'claude.answer',
  'claude.approve',
  'claude.gate',
  'claude.listModels',
  'claude.sessionsForDir',
  'claude.setEffort',
  'claude.setModel',
  'claude.setPermissionMode',
  'claude.signal',
  'config.get',
  'config.getPath',
  'config.reload',
  'config.save',
  'federation.peers',
  'fs.listDir',
  'fs.listEntries',
  'fs.read',
  'fs.readImage',
  'fs.unwatch',
  'fs.watch',
  'fs.write',
  'git.commit',
  'git.commitDiff',
  'git.commitNumstat',
  'git.diff',
  'git.log',
  'git.numstat',
  'git.push',
  'git.stage',
  'git.status',
  'git.unstage',
  // NB: layout.get/layout.set and jobs.* are HUB-LOCAL — the hub answers them
  // itself and never routes them to a provider, so they are not listed here.
  'layouts.delete',
  'layouts.list',
  'layouts.save',
  'library.list',
  'library.remove',
  'library.save',
  'providers.checkAll',
  'providers.listModels',
  'search.project',
  'sessions.attachTerminal',
  'sessions.conversation',
  'sessions.delete',
  'sessions.detachTerminal',
  'sessions.list',
  'sessions.load',
  'sessions.recent',
  'sessions.save',
  'sessions.snapshot',
  'sessions.snapshots',
  'sessions.terminalInput',
  'sessions.terminalKeepalive',
  'sessions.terminalResize',
  'terminals.create',
];

/**
 * Methods the fixture answers with an ERROR by default, so a spec meets the
 * refusing case without arranging it.
 *
 * These three were the live headless gap when this rig was written — the brain
 * registered no provider, so the promise rejected and `/app` swallowed it. web-4
 * has since ported them (`brain/claudemon.go:402-408`), which changes who is
 * refusing but not whether a refusal happens: claudemon still answers
 * `{ok:false}` when it cannot switch a running session live, a federated peer
 * can be offline, and a provider can simply be down. A rejected live-control
 * call is a permanent shape of this system, and the client must stay loud about
 * it — so the default here stays "refuse", and a spec that wants success
 * overrides it with `hub.stub()`.
 */
export const HEADLESS_GAP_METHODS = [
  'claude.setPermissionMode',
  'claude.setModel',
  'claude.setEffort',
] as const;

// ── layout builders ────────────────────────────────────────────────────────
// A seeded layout is how a spec skips the spawn dialog and lands directly in a
// workspace bound to one of the fixture sessions. Shapes follow
// `src/renderer/src/types/pane.ts` (AgentWorkspace / TabConfig / PaneConfig).

/** One pane. `type` drives which component mounts. */
export function pane(type: string, extra: Record<string, unknown> = {}) {
  return {
    id: `pane-${type}-${Math.random().toString(36).slice(2, 8)}`,
    type,
    title: type,
    ...extra,
  };
}

/** One agent workspace holding a single tab of `panes`. */
export function workspace(
  id: string,
  opts: { name?: string; cwd?: string; sessionId?: string; provider?: string; panes?: any[] } = {},
) {
  const panes = opts.panes ?? [pane('claude', { title: 'Agent' })];
  return {
    id,
    name: opts.name ?? id,
    nameSetByUser: true,
    cwd: opts.cwd ?? '/home/djtouchette/Work/worky/workspacer',
    provider: opts.provider ?? 'claude',
    sessionId: opts.sessionId,
    tabs: [{ id: `${id}-tab`, title: 'Main', panes, activePaneId: panes[0].id }],
    activeTabId: `${id}-tab`,
  };
}

/** A layout document holding `agents`, with the first one active. */
export function layoutOf(...agents: any[]): SharedLayout {
  return { agents, activeAgentId: agents[0]?.id ?? '' };
}
