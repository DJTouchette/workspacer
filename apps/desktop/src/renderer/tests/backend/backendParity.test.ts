import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import * as path from 'path';
import { createWebBackend } from '../../src/backend/webBackend';
import { LOCAL_TERMINAL, HOST_ONLY, createBridgedBackend } from '../../src/backend/bridgedBackend';

/** `.bind(ipc)` produces a new function whose name is "bound <name>". */
function isBound(fn: unknown): boolean {
  return typeof fn === 'function' && (fn as { name?: string }).name?.startsWith('bound ') === true;
}

// ─── Backend parity guard ────────────────────────────────────────────────────
// The whole renderer talks to one seam, `window.electronAPI` (typed by
// ElectronAPI). `createWebBackend` builds the FULL object for the browser/hub
// transport, so its runtime keys ARE the seam's method surface. Every method
// must be triaged into exactly one bucket, or a new method silently inherits a
// degraded stub (a hidden web-parity regression). Since ElectronAPI is a TS type
// (not reflectable), we reflect over the web backend object instead.
//
//   (a) BUS_BACKED   — rides the hub bus (a registered capability, an event
//                      subscription, or hub-core layout/publish plumbing).
//   (b) LOCAL_TERMINAL / HOST_ONLY — delegated back to preload IPC on desktop
//                      (imported verbatim from bridgedBackend.ts).
//   (c) KNOWN_STUBS  — web-degraded: returns a safe default / no-ops (the
//                      HUB-TODO + silent-stub methods in webBackend.ts).

// Keep createWebBackend from opening a real WebSocket — a no-op bus client is
// all we need to reflect over the built object's keys.
vi.mock('../../src/backend/hubBusClient', () => ({
  HubBusClient: class {
    constructor(
      readonly token: string,
      readonly busUrl?: string,
    ) {}
    start() {}
    isConnected() {
      return false;
    }
    onStatus() {
      return () => {};
    }
    onReconnect() {
      return () => {};
    }
    call() {
      return Promise.resolve({});
    }
    subscribe() {
      return () => {};
    }
    can() {
      return true;
    }
  },
}));

// Methods that ride the hub bus. Registered-capability calls, event
// subscriptions, and hub-core plumbing (layout doc, __publish) all count.
const BUS_BACKED = [
  // Discovery / model / provider
  'claudeListModels',
  'providerListModels',
  'providerCheckAll',
  // Agent control
  'claudeMessage',
  'claudeSetPermissionMode',
  'claudeSetEffort',
  'claudeSetModel',
  'claudeHandoffBrief',
  'claudeHandoffAgentBrief',
  'claudeApprove',
  'claudeAnswer',
  'claudeSignal',
  'claudeGate',
  // Files (editor)
  'readFile',
  'readImagePreview',
  'writeFile',
  'readDir',
  'watchFile',
  'searchProject',
  // Git (review pane)
  'gitStatus',
  'gitLog',
  'gitCommitDiff',
  'gitCommitNumstat',
  'gitDiff',
  'gitNumstat',
  'gitStage',
  'gitUnstage',
  'gitCommit',
  'gitPush',
  // Config
  'getConfig',
  'reloadConfig',
  'getConfigPath',
  'saveConfig',
  // Saved sessions / analytics / layouts
  'listSessions',
  'loadSession',
  'saveSession',
  'deleteSession',
  'analyticsSummary',
  'analyticsRecent',
  'layoutsList',
  'layoutsSave',
  'layoutsDelete',
  // Claude discovery / profiles / snapshots
  'claudeListSessionsForDir',
  'claudeProfilesList',
  'claudeProfilesAdd',
  'claudeProfilesUpdate',
  'claudeProfilesRemove',
  'getClaudeSession',
  'getAllClaudeSessions',
  'listRecentAgentSessions',
  'onClaudeSessionUpdate',
  // Library
  'libraryList',
  'librarySave',
  'libraryRemove',
  // App info / host fs picker
  'getCwd',
  'getSupervisorHome',
  'fsListDir',
  // Composer attachments from a client with no host filesystem: the bytes ride
  // the hub's files.upload (qualified for federation) and come back as a path
  // on the agent's machine. Absent from the preload — a pure-IPC desktop has a
  // real host path and never needs it.
  'uploadAttachment',
  // Hub plumbing (event streams, shared layout doc, publish, status)
  'onHubEvent',
  'onHubStatus',
  'getHubStatus',
  'layoutGet',
  'layoutSet',
  'onLayoutChanged',
  'hubPublish',
  // The brain has no renderer to push FACADE_OPEN_TERMINAL over IPC to, so it
  // publishes the identical payload as facade.openTerminal on the bus instead;
  // the topic is TopicGuardedBy terminals.open, enforced hub-side.
  'onFacadeOpenTerminal',
  // Remote worker nodes. nodes.list is VIEW tier so the web mirror reads it
  // straight off the bus; nodes.wake is host-authority only and the backend
  // gates the BUTTON on the connection's own tier rather than stubbing it out.
  'nodesList',
  'nodesWake',
  'nodesSleep',
  // Hub jobs (trusted-only hub-local RPCs — see HUB_CORE below)
  'jobsList',
  'jobsUpsert',
  'jobsRemove',
  'jobsRun',
  'jobsHistory',
] as const;

// Web-degraded methods: no hub RPC, they return a safe default / no-op. These
// mirror the HUB-TODO + silent stubs in webBackend.ts. Listing them explicitly
// keeps the degraded surface visible and honest (the test fails if one is
// promoted to a real bus method and left here, or removed and left here).
const KNOWN_STUBS = [
  'workflowAgentTranscript', // reads a local transcript file; null over the bus
  'workflowAgentConversation', // same
  'fileOpenExternal', // best-effort window.open(file://) on web only
  'fileShowInFolder', // reveal-in-folder impossible remotely
  'notifyQuitSaved', // no quit handshake in the browser
  'listLiveClaudeSessionIds', // boot reconcile/auto-respawn is desktop-owned; null on web
  'keepWarmHeartbeats', // keep-warm log lives in the desktop's claudemon; [] on web
  'onInAppNotification', // main-process notification mirror; web ingests notify.post bus events instead
  'notifyEscalate', // browser Notification API on web (no Electron main to escalate to)
  'onNotificationActivate', // click-through for browser-API escalations (web-local, no bus RPC)
  'getPathForFile', // Electron webUtils; a browser file has no host path → ''
  'saveClipboardImage', // the host clipboard isn't the browser user's clipboard → null
  'agentSuggestTitle', // one-shot completion on the agent's own provider CLI; null on web (the desktop titles the agent and the layout syncs)
  'onConfigChanged', // main-process config watcher; the bus has no equivalent event yet
  'federationPeers', // the web mirror talks to one hub directly; no peer link → [] (could ride hub.peer.* later)
  'federationConversation', // IMPLEMENTED on web (qualified sessions.conversation); listed here because local sessions answer null by design
  'federationPeersConfig', // peers.json lives on the hub machine; web answers null → settings render read-only
  'federationSavePeersConfig', // same — refused with a reason string
] as const;

function webBackendMethodKeys(): Set<string> {
  const api = createWebBackend('test-token') as unknown as Record<string, unknown>;
  return new Set(Object.keys(api).filter((k) => typeof api[k] === 'function'));
}

/**
 * The methods the PRELOAD exposes on window.electronAPI, parsed from
 * preload.ts's contextBridge object.
 *
 * The guard has to know this set. Anchoring "is this bucket entry stale?"
 * against the WEB backend alone meant a preload-only method was invisible to
 * the whole file — it is never in `runtime`, so no untriaged check could
 * mention it — AND declaring it in HOST_ONLY, the one place that would fix it,
 * made it look stale and turned the test red. The defect was unreportable and
 * the repair was rejected; eight remote-access methods were `undefined` in the
 * default desktop configuration for a month.
 */
function preloadMethodKeys(): Set<string> {
  const src = readFileSync(repoFile('..', '..', '..', 'main', 'preload.ts'), 'utf-8');
  const start = src.indexOf("contextBridge.exposeInMainWorld('electronAPI', {");
  if (start < 0) throw new Error('preload.ts: contextBridge.exposeInMainWorld not found');
  const keys = new Set<string>();
  for (const m of src.slice(start).matchAll(/^ {2}([A-Za-z_][A-Za-z0-9_]*)\s*:/gm)) keys.add(m[1]);
  if (keys.size < 50)
    throw new Error(`preload.ts: parsed only ${keys.size} methods — the shape changed`);
  return keys;
}

function repoFile(...segments: string[]): string {
  const here = path.dirname(fileURLToPath(import.meta.url)); // …/renderer/tests/backend
  return path.resolve(here, ...segments);
}

describe('backend parity — every ElectronAPI method is triaged into one bucket', () => {
  it('the buckets partition the web backend surface exactly (no untriaged method)', () => {
    const runtime = webBackendMethodKeys();
    const buckets: Record<string, readonly string[]> = {
      BUS_BACKED,
      LOCAL_TERMINAL,
      HOST_ONLY,
      KNOWN_STUBS,
    };

    // 1. Nothing is claimed by two buckets at once.
    const seen = new Map<string, string>();
    const overlaps: string[] = [];
    for (const [bucket, keys] of Object.entries(buckets)) {
      for (const key of keys) {
        const prior = seen.get(key);
        if (prior) overlaps.push(`${key} (in both ${prior} and ${bucket})`);
        else seen.set(key, bucket);
      }
    }
    expect(overlaps, `methods double-classified: ${overlaps.join(', ')}`).toEqual([]);

    // 2. Every runtime method is triaged into some bucket. A new electronAPI
    //    method added to webBackend.ts without classifying it fails HERE.
    const triaged = new Set(seen.keys());
    const untriaged = [...runtime].filter((k) => !triaged.has(k)).sort();
    expect(
      untriaged,
      `new/unclassified electronAPI method(s) — add each to a bucket in backendParity.test.ts: ${untriaged.join(', ')}`,
    ).toEqual([]);

    // 3. No bucket entry is stale: every declared key must still exist SOMEWHERE
    //    the bridged backend can get it from — the web backend (bus/stub) or the
    //    preload (host-only overlay). Checking only the web backend is what made
    //    the correct classification of a preload-only method fail.
    const preload = preloadMethodKeys();
    const stale = [...triaged].filter((k) => !runtime.has(k) && !preload.has(k)).sort();
    expect(
      stale,
      `bucket entries that exist on neither the web backend nor the preload: ${stale.join(', ')}`,
    ).toEqual([]);
  });

  // THE CHECK THAT WAS MISSING. createBridgedBackend starts from the web backend
  // and overlays LOCAL_TERMINAL + HOST_ONLY. A preload method in neither the web
  // backend nor either overlay list is not "degraded" in the default desktop
  // configuration — it is UNDEFINED, and every consumer feature-detects it, so
  // the feature disappears from the UI with no error, no console warning, no bus
  // event and no log.
  it('no preload method vanishes in the default (bridged) desktop backend', () => {
    const runtime = webBackendMethodKeys();
    const overlaid = new Set<string>([...LOCAL_TERMINAL, ...HOST_ONLY]);
    // `platform` is a VALUE, not a method; createBridgedBackend copies it
    // explicitly (asserted below) rather than through the overlay loop.
    const NON_METHOD = new Set(['platform']);
    const lost = [...preloadMethodKeys()]
      .filter((k) => !overlaid.has(k) && !runtime.has(k) && !NON_METHOD.has(k))
      .sort();
    expect(
      lost,
      'these preload methods are undefined on window.electronAPI in bus mode (the DEFAULT desktop launch). ' +
        'Add each to HOST_ONLY in bridgedBackend.ts, or give the web backend a stub: ' +
        lost.join(', '),
    ).toEqual([]);
  });

  it('every bridged host-only method really comes from the preload, not the web stub', () => {
    const preload = preloadMethodKeys();
    const ipc = {
      ...Object.fromEntries(
        [...preload].map((k) => [
          k,
          Object.assign(
            vi.fn(() => `ipc:${k}`),
            { __ipc: k },
          ),
        ]),
      ),
      platform: 'linux',
    } as unknown as Parameters<typeof createBridgedBackend>[0];
    const api = createBridgedBackend(ipc, 'tok', 'ws://127.0.0.1:7895/bus') as unknown as Record<
      string,
      unknown
    >;
    expect(api.platform, 'the genuine host platform must survive bus mode').toBe(
      (ipc as unknown as Record<string, unknown>).platform,
    );
    const missing = [...LOCAL_TERMINAL, ...HOST_ONLY].filter((k) => typeof api[k] !== 'function');
    expect(missing, `overlay produced no function for: ${missing.join(', ')}`).toEqual([]);
    // And the overlay must have replaced the web stub, not kept it: the `if
    // (typeof fn === 'function')` loop silently keeps the stub for a preload
    // method that was renamed.
    const notFromIpc = [...HOST_ONLY].filter(
      (k) => (api[k] as { __ipc?: string })?.__ipc === undefined && !isBound(api[k]),
    );
    expect(notFromIpc, `still the web stub, not the preload: ${notFromIpc.join(', ')}`).toEqual([]);
  });

  it('KNOWN_STUBS entries all still exist (list stays honest)', () => {
    const runtime = webBackendMethodKeys();
    for (const stub of KNOWN_STUBS) {
      expect(
        runtime.has(stub),
        `KNOWN_STUBS lists "${stub}" but the web backend has no such method`,
      ).toBe(true);
    }
  });

  it('every capability the web backend calls is a registered hub capability', () => {
    // Extract the capability names webBackend issues via client.call('<cap>', …)
    // and assert each is registered in hubCapabilities.ts (or is hub-core
    // plumbing the hub itself owns). Catches a bus method wired to a capability
    // the host never registers — a silent web-parity break.
    const webSrc = readFileSync(repoFile('..', '..', 'src', 'backend', 'webBackend.ts'), 'utf-8');
    const capSrc = readFileSync(
      repoFile('..', '..', '..', 'main', 'services', 'hubCapabilities.ts'),
      'utf-8',
    );

    // client.call('cap', …) or client.call<T>('cap', …)
    const called = new Set<string>();
    for (const m of webSrc.matchAll(/client\s*\.\s*call\s*(?:<[^>]*>)?\s*\(\s*'([^']+)'/g)) {
      called.add(m[1]);
    }
    // registerCapability('cap', …) and cat('cap', …) in hubCapabilities.ts
    const registered = new Set<string>();
    for (const m of capSrc.matchAll(/(?:registerCapability|cat)\s*\(\s*'([^']+)'/g)) {
      registered.add(m[1]);
    }
    // Hub-core surface the main process does NOT register (owned by the hub
    // daemon / bus itself), so a match against hubCapabilities.ts is not
    // expected. federation.peers is RegisterLocal'd by cmd/hub when peers are
    // configured (see internal/federation).
    const HUB_CORE = new Set([
      'layout.get',
      'layout.set',
      '__publish',
      'federation.peers',
      // Hub-owned remote-node registry (services/hub internal/nodes). Like
      // federation.peers these are provided by the HUB itself, not by a
      // desktop capability — and they are registered only when a nodes.json
      // exists, which is what makes "no provider for nodes.list" the
      // feature-absent signal the strip reads.
      'nodes.list',
      'nodes.wake',
      'nodes.sleep',
      // Hub-owned job system (services/hub/internal/jobs), trusted-only RPCs.
      'jobs.list',
      'jobs.upsert',
      'jobs.remove',
      'jobs.run',
      'jobs.history',
    ]);

    expect(called.size, 'expected to extract capability names from webBackend.ts').toBeGreaterThan(
      20,
    );
    const missing = [...called].filter((c) => !registered.has(c) && !HUB_CORE.has(c)).sort();
    expect(
      missing,
      `webBackend calls hub capabilities that hubCapabilities.ts does not register: ${missing.join(', ')}`,
    ).toEqual([]);
  });
});
