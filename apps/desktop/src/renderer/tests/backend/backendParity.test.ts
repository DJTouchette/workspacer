import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import * as path from 'path';
import { createWebBackend } from '../../src/backend/webBackend';
import { LOCAL_TERMINAL, HOST_ONLY } from '../../src/backend/bridgedBackend';

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
  'claudeSetModel',
  'claudeHandoffBrief',
  'claudeHandoffAgentBrief',
  'claudeApprove',
  'claudeAnswer',
  'claudeSignal',
  'claudeGate',
  // Files (editor)
  'readFile',
  'writeFile',
  'readDir',
  'watchFile',
  'searchProject',
  // Git (review pane)
  'gitStatus',
  'gitLog',
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
  'onClaudeSessionUpdate',
  // Library
  'libraryList',
  'librarySave',
  'libraryRemove',
  // App info / host fs picker
  'getCwd',
  'getSupervisorHome',
  'fsListDir',
  // Hub plumbing (event streams, shared layout doc, publish, status)
  'onHubEvent',
  'onHubStatus',
  'getHubStatus',
  'layoutGet',
  'layoutSet',
  'onLayoutChanged',
  'hubPublish',
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
  'listRecentAgentSessions', // no hub-bus cap for the daemon session list yet; [] on web
  'keepWarmHeartbeats', // keep-warm log lives in the desktop's claudemon; [] on web
] as const;

function webBackendMethodKeys(): Set<string> {
  const api = createWebBackend('test-token') as unknown as Record<string, unknown>;
  return new Set(Object.keys(api).filter((k) => typeof api[k] === 'function'));
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

    // 3. No bucket entry is stale: every declared key must still exist at
    //    runtime (keeps KNOWN_STUBS + the other lists honest as methods evolve).
    const stale = [...triaged].filter((k) => !runtime.has(k)).sort();
    expect(
      stale,
      `bucket entries that no longer exist on the web backend: ${stale.join(', ')}`,
    ).toEqual([]);
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
    // daemon / bus itself), so a match against hubCapabilities.ts is not expected.
    const HUB_CORE = new Set(['layout.get', 'layout.set', '__publish']);

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
