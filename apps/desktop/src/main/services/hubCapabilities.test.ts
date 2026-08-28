/**
 * Tests for registerHubCapabilities — the bus/MCP capability registry the main
 * process exposes on the hub. These caps are the remote/web/MCP control surface,
 * so the regressions that matter are behavioural, not line-coverage:
 *
 *   - agents.spawn dispatches managed (Codex/OpenCode/Pi) providers through
 *     spawnManagedAgent and Claude through spawnClaudeAgent, forwarding
 *     mcpItemIds (which this path silently dropped once before);
 *   - the SECURITY sanitization: a bus caller can NEVER auto-bypass approvals
 *     (skipPermissions / bypassPermissions / yolo are forced off);
 *   - the read-only discovery caps (providers.listModels/checkAll) and the live
 *     control pass-throughs (claude.setModel/setPermissionMode/handoffBrief);
 *   - a throwing handler surfaces a structured Error to the caller rather than
 *     crashing.
 *
 * DELEGATION MODE: this file runs with DELEGATE_CATALOG_TO_BRAIN = true, the
 * production default. Everything asserted here registers through
 * `registerCapability`, so main owns it in the mode it actually ships in. The
 * `cat`-door capabilities (fs.read/write/listEntries/listDir, library.*) are
 * NOT registered here — the Go brain answers those — and their main-side
 * handlers are exercised in the sibling hubCapabilitiesKillSwitch.test.ts,
 * which is the only file allowed to mock delegation off. Delegation-off is the
 * marked special case; it is not the baseline. This file used to mock it off
 * "for completeness", and that is precisely why a security bug in the shipping
 * path stayed invisible: the test never touched the code that runs.
 *
 * Strategy: mock ./hubClient so registerCapability records handlers into a map
 * we can invoke directly, and mock every collaborator so only the capability
 * bodies run.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';

// Capture every registered capability handler so tests can invoke them directly.
const registered = new Map<string, (params: unknown) => unknown>();
const emitToRenderer = vi.fn();
vi.mock('./hubClient', () => ({
  registerCapability: (method: string, handler: (params: unknown) => unknown) => {
    registered.set(method, handler);
  },
  emitToRenderer: (...a: unknown[]) => emitToRenderer(...a),
}));

// PRODUCTION MODE: the brain owns the catalog, so `cat(...)` registers nothing
// and only main's own registerCapability handlers exist on the bus. Do not flip
// this to false to make a test pass — a capability that is missing here is a
// capability the brain serves, and it belongs in hubCapabilitiesKillSwitch.test.ts.
vi.mock('./brainDelegation', () => ({ DELEGATE_CATALOG_TO_BRAIN: true }));

const spawnManagedAgent = vi.fn(async () => 'managed-session-id');
vi.mock('./managedSpawn', () => ({
  spawnManagedAgent: (...a: unknown[]) => spawnManagedAgent(...a),
}));

const spawnClaudeAgent = vi.fn(async () => 'claude-session-id');
vi.mock('./claudeSpawn', () => ({ spawnClaudeAgent: (...a: unknown[]) => spawnClaudeAgent(...a) }));

const createWorktree = vi.fn(async () => ({ ok: true, path: '/wt/proj-abc', branch: 'agent/x' }));
vi.mock('./worktreeService', () => ({
  createWorktree: (...a: unknown[]) => createWorktree(...a),
  worktreeInfo: vi.fn(async () => ({ isRepo: true, root: '/proj' })),
}));

const clientMock = {
  message: vi.fn(async () => ({ ok: true })),
  setPermissionMode: vi.fn(async () => ({ ok: true, mode: 'plan' })),
  setModel: vi.fn(async () => ({ ok: true })),
  handoffBrief: vi.fn(async () => ({ path: '/brief.md' })),
  listProviderModels: vi.fn(async () => ['m1', 'm2']),
  answer: vi.fn(async () => ({ ok: true, managed: true })),
  input: vi.fn(async () => undefined),
  getSubagentConversation: vi.fn(async () => ({
    seq: 2,
    items: [{ kind: 'assistant_text', text: 'child done' }],
  })),
  // Delivered by default; the spawn helper is what would flag a failure.
  takeUndeliveredFirstMessage: vi.fn(() => false),
};
vi.mock('./claudemonSessionClient', () => ({ claudemonSessionClient: clientMock }));

const notePermissionMode = vi.fn();
const getAllSnapshots = vi.fn(() => [] as unknown[]);
const getSnapshot = vi.fn(() => null as unknown);
const noteRequestedModel = vi.fn();
const clearPendingQuestions = vi.fn();
const reparentChildren = vi.fn(() => ({ moved: [] as string[], pending: [] as string[] }));
const orphanCandidates = vi.fn(() => [] as unknown[]);
vi.mock('./claudeSessionStore', async (importOriginal) => {
  // Keep the real contextTokensFromStatusLine — it's a pure helper, and the
  // agents.list statusLine-fallback test below needs the real math, not a mock.
  const actual = await importOriginal<typeof import('./claudeSessionStore')>();
  return {
    claudeSessionStore: {
      notePermissionMode: (...a: unknown[]) => notePermissionMode(...a),
      getAllSnapshots: (...a: unknown[]) => getAllSnapshots(...a),
      getSnapshot: (...a: unknown[]) => getSnapshot(...a),
      noteRequestedModel: (...a: unknown[]) => noteRequestedModel(...a),
      clearPendingQuestions: (...a: unknown[]) => clearPendingQuestions(...a),
      reparentChildren: (...a: unknown[]) => reparentChildren(...a),
      orphanCandidates: (...a: unknown[]) => orphanCandidates(...a),
    },
    contextTokensFromStatusLine: actual.contextTokensFromStatusLine,
  };
});

const checkAllProviders = vi.fn(async () => ({ codex: true }));
const resolveAgentBinary = vi.fn(() => '/bin/codex');
vi.mock('./agentProviders', () => ({
  checkAllProviders: (...a: unknown[]) => checkAllProviders(...a),
  resolveAgentBinary: (...a: unknown[]) => resolveAgentBinary(...a),
}));

const getConfig = vi.fn(() => ({ agents: { binaries: { codex: '/custom/codex' } } }));
// A real on-disk config dir: the confinement helpers canonicalize through the
// filesystem, and the config-secret deny-list below only means anything if the
// dir it guards actually exists.
const cfg = vi.hoisted(() => {
  const nodeFs = require('fs') as typeof import('fs');
  const nodeOs = require('os') as typeof import('os');
  const nodePath = require('path') as typeof import('path');
  return {
    dir: nodeFs.realpathSync(nodeFs.mkdtempSync(nodePath.join(nodeOs.tmpdir(), 'wks-cap-cfg-'))),
  };
});
const getConfigDirMock = vi.fn(() => cfg.dir);
vi.mock('./configService', () => ({
  configService: {
    getConfig: (...a: unknown[]) => getConfig(...a),
    reloadConfig: vi.fn(),
    getConfigPath: vi.fn(),
    saveConfig: vi.fn(),
  },
  getConfigDir: (...a: unknown[]) => getConfigDirMock(...a),
}));

// Handoff brief authored path — used by claude.handoffAgentBrief.
vi.mock('./agentHandoff', () => ({
  agentHandoffBrief: vi.fn(async () => ({ path: '/agent-brief.md' })),
}));

// The rest are only referenced inside handlers we do not invoke; mock them so
// importing hubCapabilities does not pull in Electron/native plumbing.
// Notification instances record their listeners so a test can fire the click
// handler — that handler is the openExternal sink under test.
const notificationHandlers = new Map<string, (...a: unknown[]) => void>();
const openExternal = vi.fn(async () => {});
vi.mock('electron', () => {
  // A plain function, not an arrow implementation: the capability calls
  // `new Notification(...)`, which an arrow can't service.
  const NotificationMock = vi.fn(function (this: Record<string, unknown>) {
    this.show = vi.fn();
    this.on = (event: string, cb: (...a: unknown[]) => void) => {
      notificationHandlers.set(event, cb);
    };
  });
  (NotificationMock as unknown as { isSupported: () => boolean }).isSupported = () => true;
  return { Notification: NotificationMock, shell: { openExternal } };
});
vi.mock('./claudeProfiles', () => ({ claudeProfiles: {} }));
vi.mock('../lib/appIcon', () => ({ appIconPath: () => undefined }));
vi.mock('./claudeModels', () => ({ listClaudeModels: vi.fn(() => []) }));
const libraryMock = { list: vi.fn(() => []), save: vi.fn(), remove: vi.fn() };
vi.mock('./libraryService', () => ({ libraryService: libraryMock }));
const notifier = {
  postInApp: vi.fn(),
  focusAgent: vi.fn(),
  focusWindow: vi.fn(),
  activateInRenderer: vi.fn(),
};
vi.mock('./agentNotifier', () => ({ agentNotifier: notifier }));
vi.mock('./sessionService', () => ({ sessionService: {} }));
vi.mock('./sessionHistory', () => ({ sessionHistory: {} }));
vi.mock('./layoutService', () => ({ layoutService: {} }));
vi.mock('./claudeSessionList', () => ({ listClaudeSessionsForDir: vi.fn() }));
const listRecentSessions = vi.fn(async () => [{ sessionId: 's1', provider: 'claude' }]);
vi.mock('./recentSessions', () => ({ listRecentSessions: () => listRecentSessions() }));
vi.mock('./fileService', () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
  listDir: vi.fn(() => ({ path: '', entries: [] })),
}));
vi.mock('./fileWatchService', () => ({ startWatch: vi.fn(), stopWatch: vi.fn() }));
vi.mock('./searchService', () => ({
  searchProject: vi.fn(() => ({ results: [], truncated: false })),
}));
// `workRoot` is part of the mock because git.diff's path guard consults it:
// gitService runs every command from the work-tree toplevel, so that — not the
// caller's cwd — is what a `path` is resolved against. Default it to the cwd
// (repo root == agent cwd); the nested-cwd test overrides it.
const workRootFor = vi.fn(async (cwd: string): Promise<string | null> => cwd);
vi.mock('./gitService', () => ({
  status: vi.fn(async () => ({ branch: 'main', files: [] })),
  workRoot: (cwd: string) => workRootFor(cwd),
  diff: vi.fn(async () => ''),
  numstat: vi.fn(async () => []),
  stage: vi.fn(async () => ''),
  unstage: vi.fn(async () => ''),
  commit: vi.fn(async () => 'committed'),
  push: vi.fn(async () => 'pushed'),
  // log / commitDiff / commitNumstat were not even mocked, which is the tell:
  // no test in this file had ever invoked those three handlers at all.
  log: vi.fn(async () => []),
  commitDiff: vi.fn(async () => ''),
  commitNumstat: vi.fn(async () => []),
}));
vi.mock('./terminalShare', () => ({}));
vi.mock('./supervisorSkill', () => ({ ensureSupervisorHome: vi.fn(() => '/home/super') }));

const { registerHubCapabilities } = await import('./hubCapabilities');
// The REAL ProgressReports singleton (over the mocked store and claudemon
// above): its refusals and its per-session budget are the capability's
// behaviour, so mocking it would leave the wiring asserting nothing.
const { progressReporter } = await import('./progressReporter');
const { searchProject } = await import('./searchService');
const gitMock = await import('./gitService');

/** Invoke a registered capability by method name. */
function call(method: string, params?: unknown): unknown {
  const handler = registered.get(method);
  if (!handler)
    throw new Error(
      `capability not registered under DELEGATE_CATALOG_TO_BRAIN=true: ${method} — ` +
        `if this is a \`cat\`-door capability the brain answers it, and its main-side ` +
        `handler belongs in hubCapabilitiesKillSwitch.test.ts`,
    );
  return handler(params);
}

beforeEach(() => {
  vi.clearAllMocks();
  registered.clear();
  clientMock.setPermissionMode.mockResolvedValue({ ok: true, mode: 'plan' });
  registerHubCapabilities();
});

describe('registerHubCapabilities — registration', () => {
  it('registers the core control + discovery capabilities', () => {
    for (const method of [
      'agents.spawn',
      'agents.sendMessage',
      'providers.listModels',
      'providers.checkAll',
      'claude.setModel',
      'claude.setPermissionMode',
      'claude.handoffBrief',
      'sessions.subagentConversation',
    ]) {
      expect(registered.has(method), `missing ${method}`).toBe(true);
    }
  });

  // sessions.recent is what makes the web client's Sessions pane non-empty:
  // sessions.snapshots only covers LIVE sessions, so the resumable list has to
  // come from the daemon-backed enrichment in recentSessions.ts. It must be a
  // real bus method (not a `cat`-delegated one) because that enrichment reads
  // main's own history DB and local transcripts.
  it('serves the daemon session list over the bus, delegating to listRecentSessions', async () => {
    expect(registered.has('sessions.recent')).toBe(true);
    await expect(call('sessions.recent')).resolves.toEqual([
      { sessionId: 's1', provider: 'claude' },
    ]);
    expect(listRecentSessions).toHaveBeenCalledTimes(1);
  });

  it('serves provider-owned subagent conversations from claudemon', async () => {
    await expect(
      call('sessions.subagentConversation', { sessionId: 'parent-1', agentId: 'child-1' }),
    ).resolves.toEqual({
      seq: 2,
      items: [{ kind: 'assistant_text', text: 'child done' }],
    });
    expect(clientMock.getSubagentConversation).toHaveBeenCalledWith('parent-1', 'child-1');
  });
});

// agents.list is the bread-and-butter "what's running?" call behind
// mcp__workspacer__list_agents, mobile, and remote. Managed providers
// (codex/opencode/pi) never populate `session.usage` — see
// claudeSessionStore's `contextTokensFromStatusLine` doc comment — so this
// must fall back to `statusLine` or every non-Claude row reports all-zero.
describe('agents.list — statusLine fallback for managed providers', () => {
  it('reads model/context/cost from usage when present (Claude-shaped session)', () => {
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'c1',
        cwd: '/proj',
        ambientState: 'idle',
        usage: {
          model: 'claude-opus-4-1',
          contextTokens: 12_000,
          contextLimit: 200_000,
          costUSD: 1.2,
        },
        statusLine: undefined,
      },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({
        sessionId: 'c1',
        model: 'claude-opus-4-1',
        contextTokens: 12_000,
        contextLimit: 200_000,
        costUSD: 1.2,
      }),
    ]);
  });

  it('falls back to statusLine when usage is null (codex-shaped session)', () => {
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'x1',
        cwd: '/proj',
        ambientState: 'idle',
        usage: null,
        statusLine: {
          modelDisplay: 'gpt-5-codex',
          contextUsedPct: 10,
          contextWindowSize: 272_000,
          costUSD: 0.4,
        },
      },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({
        sessionId: 'x1',
        model: 'gpt-5-codex',
        contextTokens: 27_200,
        contextLimit: 272_000,
        costUSD: 0.4,
      }),
    ]);
  });

  // THE FALLBACK ORDER. `statusLine.contextWindowSize` is what the PROVIDER
  // said about this session; `usage.contextLimit` is what the desktop's own
  // engine worked out from a model id. The chain preferred the computed value
  // over the reported one, and every bus client — /m, /app, remote.html,
  // wks-tui, every federated peer — inherited that ordering, which is how two
  // clients came to show different windows for the same session at the same
  // instant.
  it('prefers the PROVIDER-reported window over the desktop-computed one', () => {
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'c2',
        cwd: '/proj',
        ambientState: 'idle',
        // The desktop's engine resolved 200k off the marker-stripped
        // transcript id; the provider itself says the session holds 1M.
        usage: { model: 'claude-opus-5', contextTokens: 190_000, contextLimit: 200_000 },
        statusLine: { contextWindowSize: 1_000_000 },
      },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({ sessionId: 'c2', contextTokens: 190_000, contextLimit: 1_000_000 }),
    ]);
  });

  // ...WITH ONE EXCEPTION, and it is the whole "worker shows an absurd context
  // meter" report. Claude Code's statusLine reports `contextWindowSize: 200000`
  // even for a session spawned `opus[1m]`. A reported window the session has
  // been SEEN to exceed is not provider truth, it is a contradiction, and
  // publishing it put `{contextTokens: 356380, contextLimit: 200000}` — a 178%
  // meter — on /m, /app, wks-tui and every federated peer. Numbers below are a
  // live claudemon capture of one such worker on 2026-08-27.
  it('drops a reported window this session has been observed to exceed', () => {
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'c3',
        cwd: '/proj',
        ambientState: 'streaming',
        usage: { model: 'claude-opus-5', contextTokens: 356_380, contextLimit: 1_000_000 },
        statusLine: { contextWindowSize: 200_000 },
      },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({ sessionId: 'c3', contextTokens: 356_380, contextLimit: 1_000_000 }),
    ]);
  });

  it('keeps a reported window the session merely sits close to', () => {
    // The 2% tolerance absorbs provider rounding — 201k against a reported 200k
    // is not a contradiction, and demoting it would swap a good reading for a
    // computed one on every nearly-full session in the fleet.
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'c4',
        cwd: '/proj',
        ambientState: 'idle',
        usage: { model: 'claude-opus-5', contextTokens: 201_000, contextLimit: 1_000_000 },
        statusLine: { contextWindowSize: 200_000 },
      },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({ sessionId: 'c4', contextLimit: 200_000 }),
    ]);
  });

  it('still falls back to the reported window for a provider with no usage', () => {
    // Managed providers (codex/opencode/pi) never populate `usage`; with no
    // occupancy to contradict anything, the reported window stands.
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'c5',
        cwd: '/proj',
        ambientState: 'idle',
        statusLine: { contextWindowSize: 272_000, contextUsedPct: 10 },
      },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({ sessionId: 'c5', contextLimit: 272_000 }),
    ]);
  });

  it('falls through to usage when the provider reported no window', () => {
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'c3',
        cwd: '/proj',
        ambientState: 'idle',
        usage: { model: 'claude-opus-5', contextTokens: 1_000, contextLimit: 200_000 },
        statusLine: { contextWindowSize: undefined, costUSD: 0.1 },
      },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({ sessionId: 'c3', contextLimit: 200_000 }),
    ]);
  });

  // Honest unknown crossing the bus. A null limit is what the desktop now
  // reports for an OpenCode/Pi session, a model no table covers, or a session
  // whose observed peak disproved its claimed window. It must arrive as the
  // falsy 0 every client already reads as "no meter" — never as a 200_000
  // nobody can tell from a real one.
  it('carries a null limit across as 0, not as a guessed 200_000', () => {
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'u1',
        cwd: '/proj',
        ambientState: 'idle',
        usage: { model: 'some-new-vendor-model', contextTokens: 5_000, contextLimit: null },
        statusLine: undefined,
      },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({ sessionId: 'u1', contextTokens: 5_000, contextLimit: 0 }),
    ]);
  });

  // The freeze detector. A wedged agent reports `streaming` forever, so `state`
  // alone can never distinguish "working" from "blocked on something nobody can
  // see". `lastActivity` can: it moves only on real conversation deltas and
  // ambient transitions, never on statusLine ticks. Without it on this row the
  // only way to spot a stuck worker is to stat its worktree from outside.
  it('carries lastActivity, so a caller can tell working from wedged', () => {
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'w1',
        cwd: '/proj',
        ambientState: 'streaming',
        usage: null,
        statusLine: undefined,
        lastActivity: 1_700_000_000_000,
      },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({
        sessionId: 'w1',
        state: 'streaming',
        lastActivity: 1_700_000_000_000,
      }),
    ]);
  });

  it('reports honest zeros/nulls when neither usage nor statusLine carries data', () => {
    getAllSnapshots.mockReturnValue([
      { sessionId: 'n1', cwd: '/proj', ambientState: 'idle', usage: null, statusLine: undefined },
    ] as never);
    expect(call('agents.list')).toEqual([
      expect.objectContaining({
        sessionId: 'n1',
        model: null,
        contextTokens: 0,
        contextLimit: 0,
        costUSD: 0,
      }),
    ]);
  });

  // The successor's half of manager succession. adopt_workers reads both ids
  // out of the predecessor's handoff file — but a manager that CRASHED wrote
  // no handoff, and until this field rode the row there was no way to find the
  // workers it left pointing at a session that is gone. With it, a successor
  // groups the fleet by parentSessionId and any parent with no row of its own
  // is a dead parent still holding live children.
  it('carries parentSessionId, so orphaned workers are discoverable without a handoff file', () => {
    getAllSnapshots.mockReturnValue([
      { sessionId: 'w1', cwd: '/proj', ambientState: 'streaming', parentSessionId: 'dead-mgr' },
      { sessionId: 'w2', cwd: '/proj', ambientState: 'idle', parentSessionId: 'dead-mgr' },
      { sessionId: 'solo', cwd: '/proj', ambientState: 'idle' },
    ] as never);
    const rows = call('agents.list') as { sessionId: string; parentSessionId: string | null }[];
    expect(rows.map((r) => [r.sessionId, r.parentSessionId])).toEqual([
      ['w1', 'dead-mgr'],
      ['w2', 'dead-mgr'],
      // Explicitly null, never absent: a caller diffing parents against live
      // session ids must be able to tell "no parent" from "field not served".
      ['solo', null],
    ]);
    // The dangling-parent derivation the successor actually runs.
    const live = new Set(rows.map((r) => r.sessionId));
    const orphaned = [...new Set(rows.map((r) => r.parentSessionId))].filter(
      (p): p is string => !!p && !live.has(p),
    );
    expect(orphaned).toEqual(['dead-mgr']);
  });

  // The desktop row LAGGED the brain's on both of these: the brain answers
  // agents.list with the same enriched snapshots as sessions.snapshots, so it
  // has served label and isSupervisor since enrichSnapshot (cmd/brain
  // parity_test.go pins them, because /m titles and nests the fleet on them).
  // Convergence, not a widening — sessions.snapshots is in the SAME viewMethods
  // allowlist and already ships both through its full-snapshot spread.
  it('carries label and isSupervisor, matching the brain’s row', () => {
    getAllSnapshots.mockReturnValue([
      {
        sessionId: 'mgr',
        cwd: '/work',
        ambientState: 'idle',
        label: 'Fleet Manager',
        isSupervisor: true,
      },
      { sessionId: 'w1', cwd: '/proj', ambientState: 'idle', label: 'alpha: parser' },
      { sessionId: 'bare', cwd: '/proj', ambientState: 'idle' },
    ] as never);
    const rows = call('agents.list') as {
      sessionId: string;
      label: string | null;
      isSupervisor: boolean;
    }[];
    expect(rows.map((r) => [r.sessionId, r.label, r.isSupervisor])).toEqual([
      ['mgr', 'Fleet Manager', true],
      ['w1', 'alpha: parser', false],
      // Explicit null / false, never absent — the same rule parentSessionId
      // follows: "unnamed" must be distinguishable from "field not served".
      ['bare', null, false],
    ]);
  });
});

// The crash case. agents.reparent needs a `fromSessionId`; a manager that
// crashed wrote no handoff file to read one off and left no row to read one
// from. agents.orphans is the read that answers it — and its contract is that
// it REPORTS candidates and never picks one, because adopting the wrong group
// silently re-points another manager's workers onto the caller.
describe('agents.orphans — the successor’s read', () => {
  it('hands back every candidate, ranked, and tells the caller to choose', () => {
    orphanCandidates.mockReturnValue([
      {
        sessionId: 'dead-mgr',
        label: 'Fleet Manager',
        cwd: '/work',
        endedAt: 1_700_000_000_000,
        confirmedManager: true,
        children: [{ sessionId: 'w1', label: 'alpha: parser', cwd: '/proj', state: 'idle' }],
      },
      {
        sessionId: 'dangling',
        label: null,
        cwd: null,
        endedAt: null,
        confirmedManager: false,
        children: [{ sessionId: 'w2', label: null, cwd: '/proj', state: 'streaming' }],
      },
    ] as never);
    const out = call('agents.orphans') as { candidates: unknown[]; note: string };
    expect(out.candidates).toHaveLength(2);
    expect(out.note).toContain('1 are confirmed managers');
    expect(out.note, 'the ambiguity has to be stated, not implied').toContain('do not guess');
  });

  it('says so plainly when nothing is orphaned', () => {
    orphanCandidates.mockReturnValue([]);
    const out = call('agents.orphans') as { candidates: unknown[]; note: string };
    expect(out.candidates).toEqual([]);
    // "None" is the common answer (a clean handover, or a predecessor with
    // nothing in flight) and must not read as a failure.
    expect(out.note).toContain('Nothing is orphaned');
  });
});

// The plural call is a BACKGROUND feed: every consumer (promoteSessionSnapshots,
// useSessionSnapshots) compacts it on arrival and OverviewPane never reads
// `conversation` at all. Serializing the full transcript here paid to have it
// thrown away — over the bus that is every session's whole transcript as JSON
// on a WebSocket, on connect and on every OverviewPane refresh.
describe('sessions.snapshots — compacted before it leaves the process', () => {
  const bigSession = () => ({
    sessionId: 's1',
    cwd: '/proj',
    conversation: Array.from({ length: 50 }, (_, i) => ({
      role: i % 2 === 0 ? 'user' : 'assistant',
      content: 'x'.repeat(9000),
    })),
    completedToolCalls: Array.from({ length: 40 }, (_, i) => ({
      id: `t${i}`,
      status: 'complete',
      completedAt: i,
      input: { blob: 'y'.repeat(9000) },
    })),
  });

  it('trims the conversation tail and banks the dropped turns', async () => {
    getAllSnapshots.mockReturnValue([bigSession()] as never);
    const out = (await call('sessions.snapshots')) as Array<Record<string, any>>;

    expect(out).toHaveLength(1);
    expect(out[0].conversation).toHaveLength(12);
    // Absolute turn indices must survive the trim: 50 - 12 = 38 dropped, and
    // half of those were user sends. ClaudePane anchors on both.
    expect(out[0].conversationOffset).toBe(38);
    expect(out[0].conversationUserOffset).toBe(19);
    expect(out[0].completedToolCalls).toHaveLength(20);
  });

  it('truncates the payloads it does keep', async () => {
    getAllSnapshots.mockReturnValue([bigSession()] as never);
    const out = (await call('sessions.snapshots')) as Array<Record<string, any>>;
    // 9000 chars in, MAX_TEXT_CHARS out — the point is that the wire never
    // carries the untruncated body.
    expect(out[0].conversation[0].content.length).toBeLessThan(9000);
    expect(JSON.stringify(out[0]).length).toBeLessThan(200_000);
  });

  // The active pane reads the SINGULAR call and needs every turn; compacting it
  // would silently cut scrollback for the session the user is looking at.
  it('leaves sessions.snapshot (singular) full', async () => {
    getSnapshot.mockReturnValue(bigSession() as never);
    const out = (await call('sessions.snapshot', { sessionId: 's1' })) as Record<string, any>;
    expect(out.conversation).toHaveLength(50);
  });
});

describe('agents.spawn — dispatch', () => {
  it('routes a managed provider (codex) through spawnManagedAgent, not spawnClaudeAgent', async () => {
    const res = await call('agents.spawn', {
      provider: 'codex',
      cwd: '/proj',
      model: 'o1',
      effort: 'high',
    });

    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    expect(spawnClaudeAgent).not.toHaveBeenCalled();
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      provider: string;
      cwd: string;
      model: string;
    };
    expect(arg.provider).toBe('codex');
    expect(arg.cwd).toBe('/proj');
    expect(arg.model).toBe('o1');
    expect(res).toEqual({ sessionId: 'managed-session-id', fullAccess: false });
  });

  // mcpItemIds is CLAMPED on this path, the same way skipPermissions is, and for
  // a sharper reason. Each id resolves — through libraryService -> toClaudeEntry
  // -> buildSessionMcpConfig — into a --mcp-config entry whose `command`, `args`
  // and `env` come verbatim out of a library item, and the spawn passes
  // `--allowedTools mcp__<id>` alongside it, so the server is PRE-APPROVED and no
  // permission prompt gates it: `mcpItemIds: ['x']` is argv[0] of a host process
  // chosen by whoever wrote item x. The write side cannot be closed — a bus
  // caller reaches the item through library.save OR a plain fs.write into
  // <configDir>/library, which is a configStoreRoot by design — so the identity
  // of the SPAWNER is the only thing left to gate on. The local IPC path
  // (ipc.ts) still honours the selection.
  it('routes provider=claude (or unset) through spawnClaudeAgent and CLAMPS mcpItemIds', async () => {
    const res = await call('agents.spawn', {
      provider: 'claude',
      cwd: '/proj',
      mcpItemIds: ['srv1', 'srv2'],
    });

    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
    const arg = spawnClaudeAgent.mock.calls[0][0] as { mcpItemIds?: string[]; cwd?: string };
    expect(
      arg.mcpItemIds,
      'a bus spawn carried mcpItemIds — an MCP server definition is argv[0] of a host process, pre-approved via --allowedTools',
    ).toBeUndefined();
    // The rest of the call still rides through, so the clamp cannot be
    // "everything was dropped".
    expect(arg.cwd).toBe('/proj');
    // The mcpItemIds clamp is now REPORTED rather than only logged — see the
    // no-silent-downgrade rule in spawnResult.
    expect(res).toEqual({
      sessionId: 'claude-session-id',
      fullAccess: false,
      escalationScrubbed: ['mcpItemIds'],
    });
  });

  // The first message rides the SPAWN on all three branches, and the result
  // ACKNOWLEDGES it. `messageQueued` is what lets a dispatcher (the MCP
  // facade's confirmFirstMessage) tell "the host took the prompt" from "the
  // host does not know this field", because the second case looks identical to
  // a successful spawn while the worker sits with no task.
  it.each([
    ['managed provider', { provider: 'codex', cwd: '/proj' }, 'managed-session-id'],
    [
      'claude stream',
      { provider: 'claude', transport: 'stream', cwd: '/proj' },
      'managed-session-id',
    ],
    ['claude pty', { provider: 'claude', cwd: '/proj' }, 'claude-session-id'],
  ])('forwards `message` as firstMessage and acknowledges it — %s', async (_name, params, id) => {
    const res = await call('agents.spawn', { ...params, message: 'ship the thing' });
    const spawner =
      (params as { transport?: string }).transport === 'stream' ||
      (params as { provider?: string }).provider === 'codex'
        ? spawnManagedAgent
        : spawnClaudeAgent;
    const arg = spawner.mock.calls[0][0] as { firstMessage?: string };
    expect(arg.firstMessage).toBe('ship the thing');
    expect(res).toEqual({ sessionId: id, fullAccess: false, messageQueued: true });
  });

  // …and it reports the TRUTH, not "we passed it on". The helper already fell
  // back to a plain send and banners on total failure; answering true anyway
  // would leave the dispatcher believing it dispatched a task, which is the one
  // thing this field exists to prevent.
  it('answers messageQueued:false when the first message could not be delivered', async () => {
    clientMock.takeUndeliveredFirstMessage.mockReturnValueOnce(true);
    const res = await call('agents.spawn', {
      provider: 'codex',
      cwd: '/proj',
      message: 'ship the thing',
    });
    expect(res).toEqual({
      sessionId: 'managed-session-id',
      fullAccess: false,
      messageQueued: false,
    });
  });

  // A spawn with no message claims nothing: the result shape stays exactly what
  // every other spawn has always answered (the assertions above depend on it).
  it('makes no delivery claim when no message was sent', async () => {
    const res = await call('agents.spawn', { provider: 'codex', cwd: '/proj' });
    expect(res).toEqual({ sessionId: 'managed-session-id', fullAccess: false });
    const arg = spawnManagedAgent.mock.calls[0][0] as { firstMessage?: string };
    expect(arg.firstMessage).toBeUndefined();
  });

  it('defaults to the Claude path when no provider is given', async () => {
    await call('agents.spawn', { cwd: '/proj' });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
  });

  it("routes claude + transport 'stream' through spawnManagedAgent (standing rule: both spawn transports share the managed dispatch)", async () => {
    const res = await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      model: 'opus',
    });

    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    expect(spawnClaudeAgent).not.toHaveBeenCalled();
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      provider: string;
      transport: string;
      cwd: string;
      model: string;
    };
    expect(arg.provider).toBe('claude');
    expect(arg.transport).toBe('stream');
    expect(arg.cwd).toBe('/proj');
    expect(arg.model).toBe('opus');
    expect(res).toEqual({ sessionId: 'managed-session-id', fullAccess: false });
  });

  it("forwards profileId but CLAMPS mcpItemIds on the claude 'stream' branch", async () => {
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      profileId: 'profile-1',
      mcpItemIds: ['mcp-a', 'mcp-b'],
    });

    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      profileId?: string;
      mcpItemIds?: string[];
      scrubProfileBypass?: boolean;
    };
    // profileId still rides through — and is scrubbed downstream, which is where
    // the profile's OWN mcpItemIds are dropped (scrubBypassProfile).
    expect(arg.profileId).toBe('profile-1');
    expect(arg.scrubProfileBypass).toBe(true);
    expect(arg.mcpItemIds, 'the stream branch is the shipping default and must clamp too').toBe(
      undefined,
    );
  });

  it('forwards the hub-stamped profileGranted to both claude branches, hardened to a strict boolean', async () => {
    // The hub's sanitizeSpawnParams already deleted any caller-supplied copy —
    // by the time it reaches this provider it is trustworthy. The `=== true`
    // hardening is for a hub-bypassing local caller handing a truthy string.
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      profileId: 'work',
      profileGranted: true,
    });
    expect(
      (spawnManagedAgent.mock.calls[0][0] as { profileGranted?: boolean }).profileGranted,
    ).toBe(true);

    await call('agents.spawn', {
      provider: 'claude',
      transport: 'pty',
      cwd: '/proj',
      profileId: 'work',
      profileGranted: 'yes',
    });
    expect((spawnClaudeAgent.mock.calls[0][0] as { profileGranted?: boolean }).profileGranted).toBe(
      false,
    );

    await call('agents.spawn', { provider: 'claude', transport: 'pty', cwd: '/proj' });
    expect((spawnClaudeAgent.mock.calls[1][0] as { profileGranted?: boolean }).profileGranted).toBe(
      false,
    );
  });

  it("claude + transport 'pty' (or unset, with no config default) stays on spawnClaudeAgent", async () => {
    await call('agents.spawn', { provider: 'claude', transport: 'pty', cwd: '/proj' });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
  });

  it('worktree:true carves a worktree in main and spawns the worker THERE (ship-task isolation)', async () => {
    createWorktree.mockResolvedValueOnce({ ok: true, path: '/wt/proj-abc', branch: 'agent/x' });
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'pty',
      cwd: '/proj',
      worktree: true,
    });
    expect(createWorktree).toHaveBeenCalledTimes(1);
    expect((createWorktree.mock.calls[0][0] as { repoCwd: string }).repoCwd).toBe('/proj');
    // The worker's cwd is the worktree, not the checkout.
    expect((spawnClaudeAgent.mock.calls[0][0] as { cwd: string }).cwd).toBe('/wt/proj-abc');
  });

  it('a worktree failure falls back to cwd rather than refusing the dispatch', async () => {
    createWorktree.mockResolvedValueOnce({ ok: false, error: 'not a git repo' });
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'pty',
      cwd: '/proj',
      worktree: true,
    });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect((spawnClaudeAgent.mock.calls[0][0] as { cwd: string }).cwd).toBe('/proj');
  });

  it('no worktree is created when worktree is unset (scout / in-place work)', async () => {
    await call('agents.spawn', { provider: 'claude', transport: 'pty', cwd: '/proj' });
    expect(createWorktree).not.toHaveBeenCalled();
  });

  it('falls back to the config default (claude.transport) when the caller omits transport', async () => {
    getConfig.mockReturnValueOnce({
      agents: { binaries: { codex: '/custom/codex' } },
      claude: { transport: 'stream' },
    } as never);
    await call('agents.spawn', { provider: 'claude', cwd: '/proj' });
    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    expect(spawnClaudeAgent).not.toHaveBeenCalled();
  });

  it('sanitizes permission bypass on the claude-stream path too', async () => {
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      skipPermissions: true,
      permissionMode: 'bypassPermissions',
    });
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      skipPermissions: boolean;
      permissionMode: string | undefined;
    };
    expect(arg.skipPermissions).toBe(false);
    expect(arg.permissionMode).toBeUndefined();
  });

  it('HONORS bypass when the hub stamped yoloGranted (fleet-manager full access)', async () => {
    // yoloGranted is provenance, not a request: the hub's sanitizeSpawnParams
    // only sets it after verifying the caller's YoloAllowed grant. With it, the
    // requested skipPermissions / bypass mode rides through instead of clamping.
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      skipPermissions: true,
      permissionMode: 'bypassPermissions',
      yoloGranted: true,
    });
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      skipPermissions: boolean;
      permissionMode: string | undefined;
    };
    expect(arg.skipPermissions).toBe(true);
    expect(arg.permissionMode).toBe('bypassPermissions');
  });

  // NO SILENT DOWNGRADES (2026-08-26). The clamps above stay, but they stop
  // being invisible: every spawn answer says what the session ACTUALLY runs
  // with, and names anything the caller asked for that did not survive —
  // including what the HUB ROUTER took before the call arrived. This is the
  // fix for the reported symptom: a remote "full access" click came back
  // indistinguishable from an ask-mode spawn, with only a host log line to say
  // otherwise.
  it('REPORTS a clamped bypass in the spawn result instead of only logging it', async () => {
    const res = (await call('agents.spawn', {
      cwd: '/proj',
      skipPermissions: true,
      permissionMode: 'bypassPermissions',
    })) as { fullAccess: boolean; escalationScrubbed?: string[] };
    expect(res.fullAccess).toBe(false);
    expect(res.escalationScrubbed).toEqual(['skipPermissions', 'permissionMode']);
  });

  it('reports fullAccess:true and claims no downgrade when the hub stamped the grant', async () => {
    const res = (await call('agents.spawn', {
      cwd: '/proj',
      skipPermissions: true,
      permissionMode: 'bypassPermissions',
      yoloGranted: true,
    })) as { fullAccess: boolean; escalationScrubbed?: string[] };
    expect(res.fullAccess).toBe(true);
    expect(res.escalationScrubbed).toBeUndefined();
  });

  it("folds the ROUTER's own escalationScrubbed stamp into the answer", async () => {
    // Only the hub knows it dropped `profileId` (the calling token may not name
    // that Claude account); the provider just sees a spawn without one. The
    // stamp is how that reaches the caller.
    const res = (await call('agents.spawn', {
      cwd: '/proj',
      escalationScrubbed: ['profileId'],
    })) as { escalationScrubbed?: string[] };
    expect(res.escalationScrubbed).toEqual(['profileId']);
  });

  it('claims no downgrade for a spawn that asked for no escalation', async () => {
    // A config default that never resolves is the operator's own setting not
    // applying to an ungranted token — not something this caller lost. Counting
    // it would make every ordinary ask-mode spawn read as scrubbed.
    getConfig.mockReturnValueOnce({ claude: { skipPermissionsDefault: true } });
    const res = (await call('agents.spawn', { cwd: '/proj' })) as {
      fullAccess: boolean;
      escalationScrubbed?: string[];
    };
    expect(res.fullAccess).toBe(false);
    expect(res.escalationScrubbed).toBeUndefined();
  });

  it('a truthy-but-not-true yoloGranted does NOT unlock bypass (hub stamps a real boolean)', async () => {
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'pty',
      cwd: '/proj',
      skipPermissions: true,
      yoloGranted: 'yes',
    });
    expect(
      (spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean }).skipPermissions,
    ).toBe(false);
  });

  // The bug this dispatch was sent to find: agents.spawn is the ONLY path a
  // remote/MCP-facade Fleet Manager dispatch goes through (never the IPC
  // path), and all three of its branches used to hand-copy the spawn-options
  // object literal and silently omit `manager`/`fleetFullAccess` — the exact
  // field-drop class managedSpawnOptions.ts fixed on the IPC side. Pinned here
  // per-branch so it cannot regress quietly on any of the three again.
  describe('manager / fleetFullAccess / effort forwarding (the field-drop class this path must not repeat)', () => {
    it('forwards manager, fleetFullAccess and effort on the managed-provider branch (codex)', async () => {
      await call('agents.spawn', {
        provider: 'codex',
        cwd: '/proj',
        manager: true,
        fleetFullAccess: true,
        effort: 'high',
      });
      expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
      const arg = spawnManagedAgent.mock.calls[0][0] as {
        manager?: boolean;
        fleetFullAccess?: boolean;
        effort?: string;
      };
      expect(arg.manager).toBe(true);
      expect(arg.fleetFullAccess).toBe(true);
      expect(arg.effort).toBe('high');
    });

    it('forwards manager, fleetFullAccess and effort on the claude "stream" branch', async () => {
      await call('agents.spawn', {
        provider: 'claude',
        transport: 'stream',
        cwd: '/proj',
        manager: true,
        fleetFullAccess: true,
        effort: 'high',
      });
      expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
      const arg = spawnManagedAgent.mock.calls[0][0] as {
        manager?: boolean;
        fleetFullAccess?: boolean;
        effort?: string;
      };
      expect(arg.manager).toBe(true);
      expect(arg.fleetFullAccess).toBe(true);
      expect(arg.effort).toBe('high');
    });

    it('forwards manager, fleetFullAccess and effort on the claude "pty" branch', async () => {
      await call('agents.spawn', {
        provider: 'claude',
        transport: 'pty',
        cwd: '/proj',
        manager: true,
        fleetFullAccess: true,
        effort: 'high',
      });
      expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
      const arg = spawnClaudeAgent.mock.calls[0][0] as {
        manager?: boolean;
        fleetFullAccess?: boolean;
        effort?: string;
      };
      expect(arg.manager).toBe(true);
      expect(arg.fleetFullAccess).toBe(true);
      expect(arg.effort).toBe('high');
    });

    it('omitted manager/fleetFullAccess stay falsy rather than defaulting true (no accidental escalation)', async () => {
      await call('agents.spawn', { provider: 'codex', cwd: '/proj' });
      const arg = spawnManagedAgent.mock.calls[0][0] as {
        manager?: boolean;
        fleetFullAccess?: boolean;
      };
      expect(arg.manager).toBeUndefined();
      expect(arg.fleetFullAccess).toBeUndefined();
    });

    it('warns and drops profileId for a non-claude managed provider instead of silently discarding it', async () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      await call('agents.spawn', { provider: 'codex', cwd: '/proj', profileId: 'work' });
      const arg = spawnManagedAgent.mock.calls[0][0] as { profileId?: string };
      expect(arg.profileId).toBeUndefined();
      expect(warn).toHaveBeenCalledWith(expect.stringContaining('ignoring profileId'));
      warn.mockRestore();
    });
  });
});

describe('agents.spawn — dispatch templates', () => {
  /** A library item of kind 'dispatch' as libraryService.list would return it. */
  const tpl = (over: Record<string, unknown> = {}) => ({
    id: 'ship-task',
    scope: 'global',
    title: 'Ship task',
    kind: 'dispatch',
    body: 'SHIP: {{task}}\nDeliver: {{delivery:open a PR}}',
    resultSchema: { type: 'object', required: ['commit'] },
    path: '/cfg/library/ship-task.md',
    ...over,
  });

  it('renders the template into firstMessage and applies its default resultSchema', async () => {
    libraryMock.list.mockReturnValueOnce([tpl()] as never);
    const res = await call('agents.spawn', {
      template: 'ship-task',
      templateParams: { task: 'fix the off-by-one in parse()' },
    });
    const arg = spawnClaudeAgent.mock.calls[0][0] as {
      firstMessage?: string;
      resultSchema?: unknown;
    };
    expect(arg.firstMessage).toBe('SHIP: fix the off-by-one in parse()\nDeliver: open a PR');
    expect(arg.resultSchema).toEqual({ type: 'object', required: ['commit'] });
    // The rendered text is an ordinary first message: delivery is acknowledged,
    // AND the render is echoed back — a template spawn is the one case where the
    // dispatcher has not seen what it sent, and reading it back used to mean
    // agents.getConversation (no small-slice option, hundreds of KB).
    expect(res).toEqual({
      sessionId: 'claude-session-id',
      fullAccess: false,
      renderedMessage: 'SHIP: fix the off-by-one in parse()\nDeliver: open a PR',
      messageQueued: true,
    });
  });

  it('renderedMessage is exactly the text the worker was given', async () => {
    libraryMock.list.mockReturnValueOnce([tpl()] as never);
    const res = (await call('agents.spawn', {
      template: 'ship-task',
      templateParams: { task: 'x', delivery: 'merge locally' },
    })) as { renderedMessage?: string };
    const arg = spawnClaudeAgent.mock.calls[0][0] as { firstMessage?: string };
    // Not "a copy of the template" — the SAME string the spawn delivered, or the
    // field would verify a render that never happened.
    expect(res.renderedMessage).toBe(arg.firstMessage);
    expect(res.renderedMessage).toBe('SHIP: x\nDeliver: merge locally');
  });

  it('a plain message spawn gets NO echo — the caller wrote the text', async () => {
    const res = (await call('agents.spawn', { cwd: '/proj', message: 'do the thing' })) as Record<
      string,
      unknown
    >;
    expect(res).not.toHaveProperty('renderedMessage');
    expect(res).not.toHaveProperty('renderedMessageTruncated');
  });

  // Templates are prose a human wrote, so the cap is far above any real one. It
  // exists so a pathological template cannot bloat a tool result a manager reads
  // inside its own context window — and a clipped echo SAYS it was clipped,
  // because silently verifying a render you never saw the end of is worse than
  // no echo at all.
  it('a pathological render is truncated and flagged, never silently clipped', async () => {
    const huge = 'x'.repeat(20_000);
    libraryMock.list.mockReturnValueOnce([tpl({ body: '{{task}}' })] as never);
    const res = (await call('agents.spawn', {
      template: 'ship-task',
      templateParams: { task: huge },
    })) as { renderedMessage: string; renderedMessageTruncated?: boolean };
    const arg = spawnClaudeAgent.mock.calls[0][0] as { firstMessage?: string };
    // The WORKER still got the whole thing; only the echo is bounded.
    expect(arg.firstMessage).toHaveLength(20_000);
    expect(res.renderedMessage).toHaveLength(16_000);
    expect(res.renderedMessageTruncated).toBe(true);
  });

  it('a render at the cap is complete, and says nothing about truncation', async () => {
    libraryMock.list.mockReturnValueOnce([tpl({ body: '{{task}}' })] as never);
    const res = (await call('agents.spawn', {
      template: 'ship-task',
      templateParams: { task: 'y'.repeat(16_000) },
    })) as { renderedMessage: string; renderedMessageTruncated?: boolean };
    expect(res.renderedMessage).toHaveLength(16_000);
    expect(res.renderedMessageTruncated).toBeUndefined();
  });

  it("the CALL's own resultSchema overrides the template default", async () => {
    libraryMock.list.mockReturnValueOnce([tpl()] as never);
    await call('agents.spawn', {
      template: 'ship-task',
      templateParams: { task: 'x' },
      resultSchema: { type: 'object', required: ['verdict'] },
    });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { resultSchema?: unknown };
    expect(arg.resultSchema).toEqual({ type: 'object', required: ['verdict'] });
  });

  // THE HARD RULE: a rendered template reads finished, so an unfilled required
  // placeholder must refuse the whole spawn naming the missing param — never a
  // silent default, never a worker started on boilerplate.
  it('an unfilled required placeholder REFUSES the spawn — no worker starts', async () => {
    libraryMock.list.mockReturnValueOnce([tpl()] as never);
    await expect(call('agents.spawn', { template: 'ship-task' })).rejects.toThrow(/\{\{task\}\}/);
    expect(spawnClaudeAgent).not.toHaveBeenCalled();
    expect(spawnManagedAgent).not.toHaveBeenCalled();
  });

  it('template and message are mutually exclusive (the template IS the message)', async () => {
    // No list stub on purpose: the refusal must fire BEFORE any library read.
    await expect(
      call('agents.spawn', {
        template: 'ship-task',
        templateParams: { task: 'x' },
        message: 'also do this',
      }),
    ).rejects.toThrow(/template OR message/);
    expect(spawnClaudeAgent).not.toHaveBeenCalled();
  });

  it('an unknown template id is refused out loud', async () => {
    libraryMock.list.mockReturnValueOnce([] as never);
    await expect(
      call('agents.spawn', { template: 'nope', templateParams: { task: 'x' } }),
    ).rejects.toThrow(/no library item "nope"/);
  });

  it("only kind 'dispatch' renders — a prompt item is refused, not rendered", async () => {
    libraryMock.list.mockReturnValueOnce([tpl({ kind: 'prompt' })] as never);
    await expect(
      call('agents.spawn', { template: 'ship-task', templateParams: { task: 'x' } }),
    ).rejects.toThrow(/kind 'prompt', not 'dispatch'/);
  });

  it('templateParams without a template is refused (nothing to fill)', async () => {
    await expect(call('agents.spawn', { templateParams: { task: 'x' } })).rejects.toThrow(
      /without a template/,
    );
  });

  // THE SECURITY PROPERTY: rendering happens with the CALLER's authority
  // unchanged. A dispatch item has no spawn-argument fields by construction
  // (libraryService models none — pinned in libraryDispatch.test.ts), and even
  // a hand-forged item object carrying them changes nothing here: the spawn's
  // arguments still come from the CALL and pass the same clamps, so a template
  // cannot escalate what its caller could not.
  it('a template cannot smuggle spawn arguments — the caller clamps still apply', async () => {
    libraryMock.list.mockReturnValueOnce([
      tpl({ toolScope: 'operator', skipPermissions: true, worktree: true, cwd: '/' }),
    ] as never);
    const res = (await call('agents.spawn', {
      template: 'ship-task',
      templateParams: { task: 'x' },
      // The caller's OWN bypass request, clamped exactly as on a plain spawn.
      skipPermissions: true,
    })) as { fullAccess: boolean; escalationScrubbed?: string[] };
    const arg = spawnClaudeAgent.mock.calls[0][0] as {
      skipPermissions: boolean;
      toolScope?: string;
      cwd?: string;
    };
    expect(arg.skipPermissions).toBe(false);
    expect(res.fullAccess).toBe(false);
    expect(res.escalationScrubbed).toEqual(['skipPermissions']);
    // Nothing from the item leaked into the spawn options.
    expect(arg.toolScope).toBeUndefined();
    expect(arg.cwd).toBeUndefined();
  });
});

describe('agents.spawn — SECURITY: remote callers cannot auto-bypass approvals', () => {
  it('forces skipPermissions off even when the caller requests it (Claude path)', async () => {
    await call('agents.spawn', { cwd: '/proj', skipPermissions: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(false);
  });

  it('drops a bypassPermissions permissionMode to undefined (never auto-bypass)', async () => {
    await call('agents.spawn', { cwd: '/proj', permissionMode: 'bypassPermissions' });
    const arg = spawnClaudeAgent.mock.calls[0][0] as {
      skipPermissions: boolean;
      permissionMode: string | undefined;
    };
    expect(arg.skipPermissions).toBe(false);
    expect(arg.permissionMode).toBeUndefined();
  });

  it('drops a yolo permissionMode to undefined', async () => {
    await call('agents.spawn', { cwd: '/proj', permissionMode: 'yolo' });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { permissionMode: string | undefined };
    expect(arg.permissionMode).toBeUndefined();
  });

  it('forces skipPermissions off on the managed path too', async () => {
    await call('agents.spawn', {
      provider: 'codex',
      cwd: '/proj',
      skipPermissions: true,
      permissionMode: 'yolo',
    });
    const arg = spawnManagedAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(false);
  });

  it('preserves a safe explicit permissionMode (plan) unchanged', async () => {
    await call('agents.spawn', { cwd: '/proj', permissionMode: 'plan' });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { permissionMode: string | undefined };
    expect(arg.permissionMode).toBe('plan');
  });
});

// An OMITTED skipPermissions resolves to the config default the spawn dialog
// pre-selects (claude.skipPermissionsDefault / a bypass defaultPermissionMode),
// and the resolved value passes the SAME grant gate as an explicit request —
// honored only under the hub-stamped yoloGranted, clamped for everyone else.
// TWIN: cmd/brain spawn_skipdefault_test.go / cmd/mcp spawndefaults_test.go.
describe('agents.spawn — omitted skipPermissions resolves the config default', () => {
  const withClaudeCfg = (claude: Record<string, unknown>) =>
    getConfig.mockReturnValueOnce({
      agents: { binaries: { codex: '/custom/codex' } },
      claude,
    } as never);

  it('resolves skipPermissionsDefault:true for a granted (yoloGranted) spawn', async () => {
    withClaudeCfg({ skipPermissionsDefault: true, transport: 'pty' });
    await call('agents.spawn', { cwd: '/proj', yoloGranted: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(true);
  });

  it('resolves a bypass defaultPermissionMode the same way', async () => {
    withClaudeCfg({
      skipPermissionsDefault: false,
      defaultPermissionMode: 'bypassPermissions',
      transport: 'pty',
    });
    await call('agents.spawn', { cwd: '/proj', yoloGranted: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(true);
  });

  it('CLAMPS the config default for an ungranted caller — defaults never escalate a token', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      withClaudeCfg({ skipPermissionsDefault: true, transport: 'pty' });
      await call('agents.spawn', { cwd: '/proj' });
      const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
      expect(arg.skipPermissions).toBe(false);
      expect(warn.mock.calls.flat().join('\n')).toContain('config default');
    } finally {
      warn.mockRestore();
    }
  });

  it('an explicit false always beats the config default', async () => {
    withClaudeCfg({ skipPermissionsDefault: true, transport: 'pty' });
    await call('agents.spawn', { cwd: '/proj', skipPermissions: false, yoloGranted: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(false);
  });

  it('default off + omitted field stays approvals-on even when granted', async () => {
    withClaudeCfg({ skipPermissionsDefault: false, transport: 'pty' });
    await call('agents.spawn', { cwd: '/proj', yoloGranted: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(false);
  });

  it('the default rides the managed (claude-stream) leg too', async () => {
    withClaudeCfg({ skipPermissionsDefault: true, transport: 'stream' });
    await call('agents.spawn', { cwd: '/proj', yoloGranted: true });
    const arg = spawnManagedAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(true);
  });
});

describe('providers discovery', () => {
  // A REAL directory registered as a live agent cwd: providers.listModels' `cwd`
  // is confined to browseRoots now (claudemon runs the provider CLI in it, and
  // opencode executes <cwd>/.opencode/plugin/*.js from there), and the guard
  // canonicalizes through the filesystem, so '/proj' no longer resolves to
  // anything a root contains.
  let providerCwd: string;
  beforeEach(() => {
    providerCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-prov-')));
    getAllSnapshots.mockReturnValue([{ cwd: providerCwd }] as never);
  });
  afterEach(() => {
    getAllSnapshots.mockReturnValue([] as never);
    fs.rmSync(providerCwd, { recursive: true, force: true });
  });

  it('providers.listModels resolves the binary and queries claudemon for the provider', async () => {
    const res = await call('providers.listModels', { provider: 'codex', cwd: providerCwd });
    expect(resolveAgentBinary).toHaveBeenCalledWith('codex', '/custom/codex');
    expect(clientMock.listProviderModels).toHaveBeenCalledWith('codex', providerCwd, '/bin/codex');
    expect(res).toEqual(['m1', 'm2']);
  });

  // The cwd is not read, it is EXECUTED IN. capspec's old excuse said it merely
  // "picks which project's provider config to read"; opencode loads and RUNS
  // every <cwd>/.opencode/plugin/*.js at startup, so an unconfined cwd made a
  // capability the consent list labels "List available models" the shortest path
  // to host code execution on the whole surface.
  it('providers.listModels refuses a cwd outside the browse roots', async () => {
    clientMock.listProviderModels.mockClear();
    await expect(
      async () => await call('providers.listModels', { provider: 'opencode', cwd: '/etc' }),
    ).rejects.toThrow(/outside the allowed workspace/);
    expect(clientMock.listProviderModels).not.toHaveBeenCalled();
  });

  // An absent cwd is indistinguishable from '' on the Go side, and '' is the
  // value that absolutizes to the process working directory. Refusing it here is
  // what keeps the two providers answering the same question.
  it('providers.listModels refuses an absent cwd rather than letting the daemon pick', async () => {
    clientMock.listProviderModels.mockClear();
    await expect(
      async () => await call('providers.listModels', { provider: 'codex' }),
    ).rejects.toThrow(/outside the allowed workspace/);
    expect(clientMock.listProviderModels).not.toHaveBeenCalled();
  });

  it('providers.listModels rejects an unknown provider', async () => {
    await expect(
      async () => await call('providers.listModels', { provider: 'bogus' }),
    ).rejects.toThrow(/providers\.listModels requires/);
    expect(clientMock.listProviderModels).not.toHaveBeenCalled();
  });

  it('providers.checkAll passes the configured custom binaries through', async () => {
    const res = await call('providers.checkAll');
    expect(checkAllProviders).toHaveBeenCalledWith({ codex: '/custom/codex' });
    expect(res).toEqual({ codex: true });
  });
});

describe('claude control pass-throughs', () => {
  it('claude.setPermissionMode drives claudemon and syncs the store on success', async () => {
    const res = await call('claude.setPermissionMode', { sessionId: 's1', mode: 'plan' });
    expect(clientMock.setPermissionMode).toHaveBeenCalledWith('s1', 'plan');
    expect(notePermissionMode).toHaveBeenCalledWith('s1', 'plan');
    expect(res).toEqual({ ok: true, mode: 'plan' });
  });

  it('claude.setPermissionMode does NOT touch the store when claudemon reports failure', async () => {
    clientMock.setPermissionMode.mockResolvedValueOnce({ ok: false } as never);
    await call('claude.setPermissionMode', { sessionId: 's1', mode: 'plan' });
    expect(notePermissionMode).not.toHaveBeenCalled();
  });

  it('claude.setPermissionMode validates its params', async () => {
    await expect(
      async () => await call('claude.setPermissionMode', { sessionId: 's1' }),
    ).rejects.toThrow(/requires \{ sessionId, mode \}/);
  });

  // agents.spawn refuses to let a bus caller start an auto-approving agent —
  // "a YOLO agent must be started locally". claude.setPermissionMode reaches an
  // agent that is ALREADY RUNNING, does not ownership-check the sessionId, and
  // had no clamp at all: `mode` was validated as a non-empty string and
  // forwarded verbatim to POST /sessions/:id/permission-mode, which claudemon
  // applies for real (Shift+Tab to the bypass footer on PTY claude, the
  // adapter's auto-approve flag on codex/opencode/pi). One extra call therefore
  // undid the spawn clamp on an agent the LOCAL user had started in ask mode,
  // and agents.sendMessage drove it from there.
  for (const mode of ['bypassPermissions', 'yolo', 'dontAsk', 'auto']) {
    it(`claude.setPermissionMode refuses '${mode}' from a bus caller`, async () => {
      await expect(
        async () => await call('claude.setPermissionMode', { sessionId: 's1', mode }),
      ).rejects.toThrow(/cannot switch a running session into/);
      // A refusal that still reached the daemon would be no refusal at all.
      expect(clientMock.setPermissionMode).not.toHaveBeenCalled();
      expect(notePermissionMode).not.toHaveBeenCalled();
    });
  }

  // The floor: an allowlist that refused everything would satisfy the four cases
  // above while breaking the remote pill entirely. Tightening is not escalation.
  for (const mode of ['default', 'ask', 'acceptEdits', 'plan', 'manual']) {
    it(`claude.setPermissionMode still allows '${mode}'`, async () => {
      await call('claude.setPermissionMode', { sessionId: 's1', mode });
      expect(clientMock.setPermissionMode).toHaveBeenCalledWith('s1', mode);
    });
  }

  it('claude.setModel forwards model + effort to claudemon', async () => {
    await call('claude.setModel', { sessionId: 's1', model: 'gpt', effort: 'high' });
    expect(clientMock.setModel).toHaveBeenCalledWith('s1', 'gpt', 'high');
  });

  it('claude.setModel rejects when neither model nor effort is given', async () => {
    await expect(async () => await call('claude.setModel', { sessionId: 's1' })).rejects.toThrow(
      /requires \{ sessionId, model and\/or effort \}/,
    );
  });

  it('claude.handoffBrief forwards to claudemon', async () => {
    const res = await call('claude.handoffBrief', { sessionId: 's1' });
    expect(clientMock.handoffBrief).toHaveBeenCalledWith('s1');
    expect(res).toEqual({ path: '/brief.md' });
  });

  it('claude.handoffBrief rejects a missing sessionId', async () => {
    await expect(async () => await call('claude.handoffBrief', {})).rejects.toThrow(
      /requires \{ sessionId \}/,
    );
  });

  // G3: a codex HYBRID session is transport 'pty' (no 'stream'), but it still
  // registers the daemon's structural ask channel (start_appserver wires
  // mcp_servers.workspacer_ask for both the headless app-server and the TUI
  // that attaches to it). Routing on transport alone sent `answer` down the
  // keystroke path, which types into the TUI composer while the daemon's
  // mcp_ask shim keeps blocking for up to 6h. Routing on provider fixes it.
  it('claude.answer routes a codex HYBRID (pty-transport) session through POST /answer, not keystrokes', async () => {
    getSnapshot.mockReturnValue({ provider: 'codex', transport: 'pty' } as never);
    const res = await call('claude.answer', { sessionId: 's1', option: 2 });
    expect(clientMock.answer).toHaveBeenCalledWith('s1', {
      option: 2,
      text: undefined,
      answers: undefined,
      answerKinds: undefined,
    });
    expect(clientMock.input).not.toHaveBeenCalled();
    expect(clearPendingQuestions).toHaveBeenCalledWith('s1');
    expect(res).toEqual({ ok: true });
  });

  it('claude.answer routes a codex STREAM session through POST /answer', async () => {
    getSnapshot.mockReturnValue({ provider: 'codex', transport: 'stream' } as never);
    await call('claude.answer', { sessionId: 's1', option: 1 });
    expect(clientMock.answer).toHaveBeenCalled();
    expect(clientMock.input).not.toHaveBeenCalled();
  });

  it('claude.answer routes an opencode session through POST /answer regardless of transport', async () => {
    getSnapshot.mockReturnValue({ provider: 'opencode', transport: 'pty' } as never);
    await call('claude.answer', { sessionId: 's1', text: 'yes' });
    expect(clientMock.answer).toHaveBeenCalledWith('s1', {
      option: undefined,
      text: 'yes',
      answers: undefined,
      answerKinds: undefined,
    });
    expect(clientMock.input).not.toHaveBeenCalled();
  });

  it('claude.answer still types keystrokes for a plain claude PTY session', async () => {
    getSnapshot.mockReturnValue({ provider: 'claude', transport: 'pty' } as never);
    await call('claude.answer', { sessionId: 's1', option: 3 });
    expect(clientMock.input).toHaveBeenCalledWith('s1', '3\r');
    expect(clientMock.answer).not.toHaveBeenCalled();
  });

  it('claude.answer routes a claude STREAM session through POST /answer', async () => {
    getSnapshot.mockReturnValue({ provider: 'claude', transport: 'stream' } as never);
    await call('claude.answer', { sessionId: 's1', option: 1 });
    expect(clientMock.answer).toHaveBeenCalled();
    expect(clientMock.input).not.toHaveBeenCalled();
  });

  it('claude.answer falls back to keystrokes when the snapshot has no provider (legacy/unknown)', async () => {
    getSnapshot.mockReturnValue({ transport: 'pty' } as never);
    await call('claude.answer', { sessionId: 's1', option: 1 });
    expect(clientMock.input).toHaveBeenCalledWith('s1', '1\r');
    expect(clientMock.answer).not.toHaveBeenCalled();
  });
});

describe('agents.sendMessage', () => {
  it('forwards to claudemon.message and returns ok', async () => {
    const res = await call('agents.sendMessage', { sessionId: 's1', text: 'hi' });
    expect(clientMock.message).toHaveBeenCalledWith('s1', 'hi');
    expect(res).toEqual({ ok: true });
  });

  it('surfaces a not-accepting-input rejection when claudemon returns ok:false', async () => {
    clientMock.message.mockResolvedValueOnce({ ok: false, mode: 'Approval' } as never);
    await expect(
      async () => await call('agents.sendMessage', { sessionId: 's1', text: 'hi' }),
    ).rejects.toThrow(/not accepting input.*Approval/);
  });

  it('validates params before hitting claudemon', async () => {
    await expect(async () => await call('agents.sendMessage', { sessionId: 's1' })).rejects.toThrow(
      /requires \{ sessionId, text \}/,
    );
    expect(clientMock.message).not.toHaveBeenCalled();
  });
});

// agents.reportProgress is the WORKER's half of the fleet wake channel, and the
// wiring is what makes it exist at all: the service (progressReports.ts) landed
// on master with no caller. These assert the two ends the wiring is responsible
// for — the recipient is derived, never named, and the wake that arrives is a
// 'progress' one, which is the whole reason the kind exists (a manager that
// books a self-report as a finish has recorded work that has not landed).
describe('agents.reportProgress', () => {
  const worker = {
    sessionId: 'w-1',
    cwd: '/w/alpha',
    label: 'alpha: parser',
    parentSessionId: 'mgr-1',
  };
  const manager = { sessionId: 'mgr-1', cwd: '/w', label: 'Fleet Manager' };

  beforeEach(() => {
    progressReporter.reset();
    getAllSnapshots.mockReturnValue([worker, manager]);
  });

  it('delivers the note to the caller’s PARENT as a progress wake, not a finish', async () => {
    const res = await call('agents.reportProgress', {
      callerSessionId: 'w-1',
      note: 'phase 1 landed; starting the migration',
    });
    expect(res).toEqual({ deliveredTo: 'mgr-1' });

    const [to, text] = clientMock.message.mock.calls[0] as [string, string];
    expect(to).toBe('mgr-1');
    expect(text).toContain('STILL RUNNING');
    expect(text).not.toContain('finished');
    expect(text).toContain('reports: phase 1 landed; starting the migration');
    expect(text).toContain('alpha: parser (session:w-1, cwd /w/alpha)');
  });

  it('marks a blocked worker so the manager can tell it from an FYI', async () => {
    await call('agents.reportProgress', {
      callerSessionId: 'w-1',
      note: 'the approach you gave me is wrong',
      needsDecision: true,
    });
    expect(clientMock.message.mock.calls[0][1]).toContain('NEEDS A DECISION');
  });

  // The containment: there is no recipient param, so a caller the host cannot
  // identify has nowhere for its note to go — and it is TOLD so rather than
  // having the message dropped, because a worker that believes it reported and
  // did not is the failure this channel exists to prevent.
  it('refuses rather than guessing when the caller has no identity', async () => {
    await expect(
      async () => await call('agents.reportProgress', { note: 'hello' }),
    ).rejects.toThrow(/could not identify your session/);
    expect(clientMock.message).not.toHaveBeenCalled();
  });

  it('refuses a caller with no parent — nothing dispatched it', async () => {
    await expect(
      async () => await call('agents.reportProgress', { callerSessionId: 'mgr-1', note: 'hello' }),
    ).rejects.toThrow(/no parent session/);
    expect(clientMock.message).not.toHaveBeenCalled();
  });
});

// agents.reparent is the manager-succession verb, and the wiring is the whole
// point: claudeSessionStore.reparentChildren landed fully tested with NO caller,
// so a replaced Fleet Manager still orphaned every dispatch it had in flight.
// These assert the seam — the two ids reach the store in the right order, and
// the store's refusals reach the caller instead of being smoothed over.
describe('agents.reparent', () => {
  beforeEach(() => {
    reparentChildren.mockReset();
    reparentChildren.mockReturnValue({ moved: [], pending: [] });
  });

  it('hands the store (outgoing, successor) in that order and says where the wakes go now', () => {
    reparentChildren.mockReturnValue({ moved: ['w-1', 'w-2'], pending: ['w-3'] });
    const res = call('agents.reparent', { fromSessionId: 'old-mgr', toSessionId: 'new-mgr' }) as {
      moved: string[];
      pending: string[];
      note: string;
    };
    expect(reparentChildren).toHaveBeenCalledWith('old-mgr', 'new-mgr');
    expect(res.moved).toEqual(['w-1', 'w-2']);
    expect(res.pending).toEqual(['w-3']);
    expect(res.note).toContain('3 dispatch(es) now report to new-mgr');
  });

  // "Nothing moved" is the answer when the predecessor had nothing left in
  // flight, and a successor that reads it as a failure would fall back to the
  // hand-reconciliation ritual this verb exists to delete.
  it('reports an empty move as a real answer, not an error', () => {
    const res = call('agents.reparent', { fromSessionId: 'old-mgr', toSessionId: 'new-mgr' }) as {
      moved: string[];
      note: string;
    };
    expect(res.moved).toEqual([]);
    expect(res.note).toContain('Nothing was still parented to old-mgr');
  });

  it('requires both ids — a half-named move is refused before the store is touched', () => {
    expect(() => call('agents.reparent', { fromSessionId: 'old-mgr' })).toThrow(
      /requires \{ fromSessionId, toSessionId \}/,
    );
    expect(() => call('agents.reparent', { toSessionId: 'new-mgr' })).toThrow(
      /requires \{ fromSessionId, toSessionId \}/,
    );
    expect(reparentChildren).not.toHaveBeenCalled();
  });

  // The store refuses a successor no wake could reach (unknown, ended, not a
  // manager). That refusal is the safety property; swallowing it here would
  // report a successful adoption of workers that had just been silenced.
  it('surfaces the store’s refusal verbatim', () => {
    reparentChildren.mockImplementation(() => {
      throw new Error('reparent_children: new-mgr is not a manager (isSupervisor)');
    });
    expect(() =>
      call('agents.reparent', { fromSessionId: 'old-mgr', toSessionId: 'new-mgr' }),
    ).toThrow(/is not a manager/);
  });
});

describe('error propagation', () => {
  it('a handler throwing (validation) surfaces a structured Error, not a crash', async () => {
    // The bus caller invokes the handler; an invalid call must reject with a
    // real Error whose message the bus can serialize — never throw synchronously
    // in a way that kills the provider.
    await expect(async () => await call('claude.setModel', {})).rejects.toBeInstanceOf(Error);
  });

  it('propagates a rejection from the underlying spawn (does not swallow it)', async () => {
    spawnClaudeAgent.mockRejectedValueOnce(new Error('spawn boom'));
    await expect(async () => await call('agents.spawn', { cwd: '/proj' })).rejects.toThrow(
      'spawn boom',
    );
  });

  it('propagates a rejection from claudemon.setModel', async () => {
    clientMock.setModel.mockRejectedValueOnce(new Error('daemon down'));
    await expect(
      async () => await call('claude.setModel', { sessionId: 's1', model: 'x' }),
    ).rejects.toThrow('daemon down');
  });
});

describe('search.project cwd confinement', () => {
  // search.project is registerCapability, not `cat`: main answers it in
  // production, so its confinement is asserted here rather than in the
  // kill-switch file the fs.*/library.* cases moved to.
  //
  // A real temp dir stands in for a live agent's cwd — the confinement helpers
  // canonicalize via the real filesystem, so the roots must exist.
  let agentCwd: string;
  beforeEach(() => {
    agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-agent-')));
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
  });

  it('search.project denies a cwd outside the workspace', () => {
    expect(() => call('search.project', { query: 'x', cwd: '/etc' })).toThrow(
      /outside the allowed workspace/,
    );
    expect(searchProject).not.toHaveBeenCalled();
  });

  it('search.project allows a cwd inside a live agent cwd', () => {
    expect(() => call('search.project', { query: 'x', cwd: agentCwd })).not.toThrow();
    expect(searchProject).toHaveBeenCalled();
  });
});

describe('notifications.post — external URL scheme check', () => {
  function clickWith(url: string): void {
    call('notifications.post', { title: 't', url });
    const onClick = notificationHandlers.get('click');
    expect(onClick).toBeDefined();
    onClick!();
  }

  it('opens an https URL', () => {
    clickWith('https://example.com/build/42');
    expect(openExternal).toHaveBeenCalledWith('https://example.com/build/42');
  });

  it('refuses a file:// URL (shell.openExternal would launch it)', () => {
    clickWith('file:///etc/passwd');
    expect(openExternal).not.toHaveBeenCalled();
  });

  it('refuses a custom-protocol URL', () => {
    clickWith('vscode://file/etc/shadow');
    expect(openExternal).not.toHaveBeenCalled();
  });
});

describe('notifications.post — click targets', () => {
  // The capability has always ACCEPTED a pane target; until the facade could
  // send one, nothing exercised it, and the click handler quietly ignored it.
  it('routes a pane target (with its section) to the renderer', () => {
    call('notifications.post', {
      title: 'Job proposed: Nightly sync',
      paneType: 'settings',
      paneSection: 'jobs',
    });
    notificationHandlers.get('click')!();

    expect(notifier.activateInRenderer).toHaveBeenCalledTimes(1);
    const n = notifier.activateInRenderer.mock.calls[0][0];
    expect(n.paneType).toBe('settings');
    expect(n.paneSection).toBe('jobs');
    // Same object the center holds, so the two can't disagree about where it
    // points, and clicking marks it read.
    expect(notifier.postInApp.mock.calls[0][0].id).toBe(n.id);
  });

  it('routes a session target to the renderer', () => {
    call('notifications.post', { title: 't', sessionId: 's1' });
    notificationHandlers.get('click')!();
    expect(notifier.activateInRenderer).toHaveBeenCalledTimes(1);
    expect(openExternal).not.toHaveBeenCalled();
  });
});

describe('git.* cwd confinement', () => {
  // The review-pane git surface moved from claudemon to the host; its bus caps are
  // now the remote-reachable entry point, so a caller-supplied cwd must be confined
  // to the live agent cwds (the same workspace roots as fs.*), not any host repo.
  let agentCwd: string;
  beforeEach(() => {
    agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-git-')));
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
    workRootFor.mockImplementation(async (cwd: string) => cwd);
  });

  it('git.commit runs when cwd is a live agent cwd', async () => {
    await call('git.commit', { cwd: agentCwd, message: 'wip' });
    expect(gitMock.commit).toHaveBeenCalledWith(agentCwd, 'wip');
  });

  it('git.commit is denied for a cwd outside the workspace', async () => {
    expect(() => call('git.commit', { cwd: '/tmp/some-other-repo', message: 'wip' })).toThrow(
      /outside the allowed workspace/,
    );
    expect(gitMock.commit).not.toHaveBeenCalled();
  });

  it('git.push is denied for a cwd outside the workspace', () => {
    expect(() => call('git.push', { cwd: os.homedir() })).toThrow(/outside the allowed workspace/);
    expect(gitMock.push).not.toHaveBeenCalled();
  });

  it('git.status (read) is also confined to the workspace', () => {
    expect(() => call('git.status', { cwd: '/etc' })).toThrow(/outside the allowed workspace/);
    expect(gitMock.status).not.toHaveBeenCalled();
  });

  it('git.status runs for a live agent cwd', async () => {
    await call('git.status', { cwd: agentCwd });
    expect(gitMock.status).toHaveBeenCalledWith(agentCwd);
  });

  // EVERY git.* capability, not the four somebody happened to write a test for.
  //
  // All ten take a caller-supplied absolute `cwd` and guardGitCwd is the only
  // thing confining them; capspec excuses all of them from PathParam on exactly
  // that ground. Yet only git.status / git.commit / git.push / git.diff were
  // ever named by a test, so the guardGitCwd() call could be deleted from
  // git.log, git.numstat, git.commitDiff, git.commitNumstat, git.stage and
  // git.unstage with the whole Go and desktop suites staying green — handing
  // every bus client (web / remote / MCP / any trusted connection) the diffs,
  // commit messages and file contents of any repo on the host, and write access
  // to any index. Table-driven so a new git.* handler has to be added here to
  // pass the completeness assertion at the end.
  const gitMethods: {
    method: string;
    params: Record<string, unknown>;
    fn: () => { mock: { calls: unknown[][] } };
  }[] = [
    { method: 'git.status', params: {}, fn: () => gitMock.status as never },
    { method: 'git.log', params: { limit: 5 }, fn: () => gitMock.log as never },
    { method: 'git.diff', params: {}, fn: () => gitMock.diff as never },
    { method: 'git.numstat', params: {}, fn: () => gitMock.numstat as never },
    { method: 'git.commitDiff', params: { hash: 'abc123' }, fn: () => gitMock.commitDiff as never },
    {
      method: 'git.commitNumstat',
      params: { hash: 'abc123' },
      fn: () => gitMock.commitNumstat as never,
    },
    { method: 'git.stage', params: { path: 'a.txt' }, fn: () => gitMock.stage as never },
    { method: 'git.unstage', params: { path: 'a.txt' }, fn: () => gitMock.unstage as never },
    { method: 'git.commit', params: { message: 'wip' }, fn: () => gitMock.commit as never },
    { method: 'git.push', params: {}, fn: () => gitMock.push as never },
  ];

  /** Some handlers are async (git.diff), so a refusal surfaces as a rejection
   *  rather than a synchronous throw. Normalize both into a message. */
  async function refusal(method: string, params: unknown): Promise<string> {
    try {
      await call(method, params);
    } catch (err) {
      return err instanceof Error ? err.message : String(err);
    }
    throw new Error(`${method} returned instead of refusing`);
  }

  for (const { method, params, fn } of gitMethods) {
    it(`${method} is confined to the workspace roots`, async () => {
      const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-other-repo-')));
      expect(await refusal(method, { cwd: outside, ...params })).toMatch(
        /outside the allowed workspace/,
      );
      // A refusal that still ran the command would be worse than no guard.
      expect(fn().mock.calls, `${method} must not reach gitService`).toHaveLength(0);
    });

    it(`${method} refuses a cwd that leaves the roots through a symlink`, async () => {
      const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-other-repo-')));
      // The reason the guard canonicalizes rather than string-prefixing: the
      // link SITS inside the allowed root.
      const link = path.join(agentCwd, 'escape');
      fs.symlinkSync(outside, link);
      expect(await refusal(method, { cwd: link, ...params })).toMatch(
        /outside the allowed workspace/,
      );
      expect(fn().mock.calls, `${method} must not reach gitService`).toHaveLength(0);
    });

    it(`${method} runs for a live agent cwd, and gets the CANONICAL path`, async () => {
      // The floor: a guard that refused everything would satisfy both cases
      // above. And what reaches gitService has to be the resolved directory —
      // the checked path and the directory git runs in cannot differ.
      const inner = path.join(agentCwd, 'real');
      fs.mkdirSync(inner, { recursive: true });
      fs.symlinkSync(inner, path.join(agentCwd, 'alias'));
      await call(method, { cwd: path.join(agentCwd, 'alias'), ...params });
      expect(fn().mock.calls[0]?.[0], `${method} must receive the canonical cwd`).toBe(inner);
    });
  }

  it('covers every git.* capability the provider registers', () => {
    // Without this, adding git.blame with no entry above would leave it as
    // unpinned as the six this block was written for.
    const registeredGit = [...registered.keys()].filter((m) => m.startsWith('git.')).sort();
    expect(gitMethods.map((g) => g.method).sort()).toEqual(registeredGit);
  });

  // git.diff's `path` is not just a pathspec: with untracked:true gitService
  // hands it to `git diff --no-index -- /dev/null <path>`, where git treats it as
  // a filesystem operand. A legal cwd plus an escaping path therefore read any
  // file on the host as an all-added diff until the path was confined too.
  it('git.diff denies an absolute path outside the repo (untracked --no-index operand)', async () => {
    await expect(
      call('git.diff', { cwd: agentCwd, path: '/etc/shadow', untracked: true }),
    ).rejects.toThrow(/outside the allowed workspace/);
    expect(gitMock.diff).not.toHaveBeenCalled();
  });

  it('git.diff denies a traversal path that escapes the repo', async () => {
    await expect(
      call('git.diff', { cwd: agentCwd, path: '../../../etc/passwd', untracked: true }),
    ).rejects.toThrow(/outside the allowed workspace/);
    expect(gitMock.diff).not.toHaveBeenCalled();
  });

  it('git.diff still allows a repo-relative path inside the agent cwd', async () => {
    await call('git.diff', { cwd: agentCwd, path: 'src/new.ts', untracked: true });
    expect(gitMock.diff).toHaveBeenCalledWith(agentCwd, 'src/new.ts', undefined, true);
  });

  // The guard has to measure `path` the way git will: gitService anchors every
  // command at `rev-parse --show-toplevel`, so with the agent cwd nested in a
  // monorepo (the normal case) a path resolved against the agent cwd names a
  // different file than the one git opens.
  describe('with the agent cwd nested below the repo root', () => {
    let repoRoot: string;
    beforeEach(() => {
      repoRoot = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-repo-')));
      agentCwd = path.join(repoRoot, 'apps', 'desktop');
      fs.mkdirSync(agentCwd, { recursive: true });
      getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
      workRootFor.mockImplementation(async () => repoRoot);
    });
    afterEach(() => fs.rmSync(repoRoot, { recursive: true, force: true }));

    it('refuses a ../-path that measuring from the agent cwd would wave through', async () => {
      // Measured from the agent cwd this names a file inside a SECOND live
      // agent's cwd — inside a workspace root, so a cwd-based check admits it.
      // git runs two levels shallower, from repoRoot, where the same string
      // normalizes somewhere else entirely and outside every repo: the check and
      // the read were looking at different files.
      const otherAgent = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-other-')));
      getAllSnapshots.mockReturnValue([{ cwd: agentCwd }, { cwd: otherAgent }] as never);
      const rel = path.relative(agentCwd, path.join(otherAgent, 'secret.env'));

      await expect(call('git.diff', { cwd: agentCwd, path: rel, untracked: true })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.diff).not.toHaveBeenCalled();
      fs.rmSync(otherAgent, { recursive: true, force: true });
    });

    it('refuses a path that climbs out of the repo root', async () => {
      await expect(
        call('git.diff', { cwd: agentCwd, path: '../../../etc/passwd', untracked: true }),
      ).rejects.toThrow(/outside the allowed workspace/);
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    // The `untracked` leg is a different capability wearing the same name.
    // `git diff --no-index -- /dev/null <path>` renders ANY readable file as an
    // all-added diff — gitignored, untracked and tracked-but-unmodified alike —
    // none of which a path-less diff shows. So the "confining to the repo
    // concedes nothing" argument does not cover it, and the work-tree root is a
    // DERIVED directory nothing ever checked against the allow-list: an agent
    // cwd of <repo>/apps/desktop read <repo>/services/hub/.env this way, a file
    // fs.read and fs.watch refuse for the same caller.
    it('refuses an untracked read of a sibling subtree the tracked pathspec allows', async () => {
      await expect(
        call('git.diff', { cwd: agentCwd, path: 'services/hub/.env', untracked: true }),
      ).rejects.toThrow(/outside the allowed workspace/);
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    it('still allows an untracked path INSIDE the agent cwd', async () => {
      const rel = path.relative(repoRoot, path.join(agentCwd, 'src', 'new.ts'));
      await call('git.diff', { cwd: agentCwd, path: rel, untracked: true });
      expect(gitMock.diff).toHaveBeenCalledWith(agentCwd, rel, undefined, true);
    });

    // ── the staging leg ────────────────────────────────────────────────
    //
    // git.stage and a path-less git.diff{staged} COMPOSE into exfiltration that
    // neither is on its own. `git add` runs from the DERIVED work-tree root
    // (rev-parse --show-toplevel, a directory nothing ever checked against the
    // allow-list), and `path` reached gitService with no guard at all — so a
    // root-relative pathspec, or NO pathspec at all (`git add -A` over the whole
    // repository), put files outside every allowed root into the index, where
    // `git diff --staged` renders each of them as an all-added diff with full
    // content because they are not in HEAD. git.commit persists it,
    // git.commitDiff hands it back, git.push publishes it.
    it('git.stage refuses a sibling-subtree pathspec the tracked diff would allow', async () => {
      await expect(call('git.stage', { cwd: agentCwd, path: 'services/hub/.env' })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.stage).not.toHaveBeenCalled();
    });

    it('git.stage refuses an absolute pathspec outside the repo', async () => {
      await expect(call('git.stage', { cwd: agentCwd, path: '/etc/shadow' })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.stage).not.toHaveBeenCalled();
    });

    // The path-LESS form is the half a per-path guard cannot reach: `git add -A`
    // from the root stages the sibling subtree without naming it.
    it('git.stage with no path stages the guarded cwd, not the whole repository', async () => {
      await call('git.stage', { cwd: agentCwd });
      expect(gitMock.stage).toHaveBeenCalledWith(agentCwd, 'apps/desktop');
    });

    it('git.unstage with no path is bounded the same way', async () => {
      await call('git.unstage', { cwd: agentCwd });
      expect(gitMock.unstage).toHaveBeenCalledWith(agentCwd, 'apps/desktop');
    });

    it('git.unstage refuses a sibling-subtree pathspec', async () => {
      await expect(
        call('git.unstage', { cwd: agentCwd, path: 'services/hub/.env' }),
      ).rejects.toThrow(/outside the allowed workspace/);
      expect(gitMock.unstage).not.toHaveBeenCalled();
    });

    it('git.stage still stages a path inside the agent cwd, root-relative', async () => {
      const rel = path.relative(repoRoot, path.join(agentCwd, 'src', 'new.ts'));
      await call('git.stage', { cwd: agentCwd, path: rel });
      expect(gitMock.stage).toHaveBeenCalledWith(agentCwd, rel);
    });

    // ── the TRACKED leg, where the work-tree-root assertion is the ONLY guard ──
    //
    // git.diff passes `untracked ? [workspaceRoots()] : []` as extraRootSets, so
    // on a TRACKED diff the extra sets are EMPTY and the single
    // `assertPathAllowed(cap, anchored, [root])` is the whole boundary — its
    // containment half and its secret gate both. Every git-pathspec test above
    // rides an extraRootSet (untracked, stage, unstage), so replacing that line
    // with `const canonicalFile = anchored;` left 88 files / 1379 tests green.
    it('a TRACKED diff refuses a pathspec that climbs out of the work-tree root', async () => {
      await expect(
        call('git.diff', { cwd: agentCwd, path: '../../../etc/passwd' }),
      ).rejects.toThrow(/outside the allowed workspace/);
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    it('a TRACKED diff refuses an absolute pathspec outside the work-tree root', async () => {
      await expect(call('git.diff', { cwd: agentCwd, path: '/etc/shadow' })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    it('a TRACKED diff refuses a credential the secret gate names', async () => {
      // The gate only ever runs INSIDE assertPathAllowed. A modified ~/.gitconfig
      // routinely carries credential-helper settings and url.<base>.insteadOf
      // tokens, and `.bus-token` / `.git/config` are the same shape.
      for (const p of ['.git/config', '.bus-token', '.gitconfig']) {
        await expect(call('git.diff', { cwd: agentCwd, path: p })).rejects.toThrow(
          /outside the allowed workspace/,
        );
      }
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    // BINDING DECISION 2 on the OPERAND: what git receives is a function of the
    // CANONICAL path, never of the caller's string. `return filePath` survived
    // the whole suite because every test above happens to pass a string that is
    // already the answer.
    it('hands git the pathspec derived from the canonical path, not the caller string', async () => {
      await call('git.diff', { cwd: agentCwd, path: path.join(repoRoot, 'services', 'a.go') });
      expect(gitMock.diff).toHaveBeenCalledWith(agentCwd, 'services/a.go', undefined, undefined);
      gitMock.diff.mockClear();
      await call('git.diff', { cwd: agentCwd, path: 'services/./hub/../a.go' });
      expect(gitMock.diff).toHaveBeenCalledWith(agentCwd, 'services/a.go', undefined, undefined);
    });

    // cwdPathspec's own fail-closed precondition. Its comment says the assertion
    // "proves the cwd really is at-or-inside the derived root before path.relative
    // is trusted to describe it (a `..` result would be a pathspec pointing OUT of
    // the repo)". The helper's OUTPUT is pinned by the two tests above; the
    // precondition was pinned by nothing, and the work-tree root is DERIVED — a
    // gitfile, GIT_WORK_TREE or a submodule can make it a directory that does not
    // contain the cwd at all.
    it('git.stage with no path refuses a work-tree root that does not contain the cwd', async () => {
      const elsewhere = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-elsewhere-')));
      workRootFor.mockImplementation(async () => elsewhere);
      await expect(call('git.stage', { cwd: agentCwd })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.stage).not.toHaveBeenCalled();
      await expect(call('git.unstage', { cwd: agentCwd })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.unstage).not.toHaveBeenCalled();
      fs.rmSync(elsewhere, { recursive: true, force: true });
    });

    it('still allows a root-relative path in a sibling subtree (what git.status hands back)', async () => {
      // git.status prints repo-root-relative paths for the WHOLE repo, and the
      // review pane feeds them straight back; refusing them because they sit
      // outside the agent cwd would break review in every monorepo.
      await call('git.diff', { cwd: agentCwd, path: 'services/hub/main.go' });
      expect(gitMock.diff).toHaveBeenCalledWith(
        agentCwd,
        'services/hub/main.go',
        undefined,
        undefined,
      );
    });
  });
});

describe('terminals.open — the visible-terminal seam', () => {
  beforeEach(() => emitToRenderer.mockClear());

  it('pushes a FACADE_OPEN_TERMINAL event to the renderer with the caller fields intact', () => {
    const res = call('terminals.open', {
      cwd: os.tmpdir(),
      command: 'npm run dev',
      label: 'preheat dev server',
      parentSessionId: 'MGR',
    });
    expect(res).toEqual({ ok: true });
    expect(emitToRenderer).toHaveBeenCalledTimes(1);
    const [channel, payload] = emitToRenderer.mock.calls[0] as [string, Record<string, unknown>];
    // The renderer opens the pane off this channel (IPC.FACADE_OPEN_TERMINAL).
    expect(channel).toBe('terminal:facade-open');
    expect(payload).toMatchObject({
      command: 'npm run dev',
      label: 'preheat dev server',
      parentSessionId: 'MGR',
    });
    // cwd is normalized (an existing dir survives); it is always a string.
    expect(typeof payload.cwd).toBe('string');
  });

  it('drops non-string fields rather than forwarding junk', () => {
    call('terminals.open', {
      cwd: os.tmpdir(),
      command: 123,
      label: { nope: true },
      parentSessionId: 'MGR',
    });
    const [, payload] = emitToRenderer.mock.calls[0] as [string, Record<string, unknown>];
    expect(payload.command).toBeUndefined();
    expect(payload.label).toBeUndefined();
    expect(payload.parentSessionId).toBe('MGR');
  });
});

describe('brief.append — append from a worker RESULT', () => {
  // The two halves of a brief line: the manager's judgement (irreplaceable) and
  // the worker's mechanical facts (already reported verbatim in its wks-result).
  // With the optional params present the host writes the second half, and — the
  // reason this landed — writes the `session:<id>` reference correctly, instead
  // of a manager retyping a nickname like `session:6a-round2` into a brief and
  // repairing the dead link by hand afterwards.
  const LIVE = 'c03bd8ce-1f4a-4b2c-9d3e-0123456789ab';
  let agentCwd: string;

  const readBrief = (): string =>
    fs.readFileSync(path.join(agentCwd, '.workspacer', 'brief.md'), 'utf-8');

  beforeEach(() => {
    agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-brief-')));
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd, sessionId: LIVE, mode: 'running' }] as never);
  });

  it('PLAIN append is byte-for-byte unchanged when the new params are absent', () => {
    call('brief.append', { project: agentCwd, section: 'Recently', line: '2026-08-21  shipped X' });
    // Not "starts with", not "contains": the exact line the caller composed,
    // with no date, no facts and no reference bolted on.
    expect(readBrief()).toContain('\n- 2026-08-21  shipped X\n');
    expect(readBrief()).not.toMatch(/session:/);
  });

  it('composes date + sentence + facts + reference from a result', () => {
    call('brief.append', {
      project: agentCwd,
      section: 'Recently',
      line: 'the parser no longer allocates per token',
      sessionId: LIVE,
      result: {
        commit: 'abc1234',
        filesChanged: ['src/parser.ts', 'src/lexer.ts'],
        checksRun: ['vitest'],
        caveats: ['the migration is not reversible'],
      },
    });
    const written = readBrief();
    expect(written).toMatch(/^- \d{4}-\d{2}-\d{2} {2}the parser no longer allocates/m);
    expect(written).toContain('commit: abc1234');
    expect(written).toContain('filesChanged: src/parser.ts, src/lexer.ts');
    // Never silently dropped, whatever else is capped.
    expect(written).toContain('caveats: the migration is not reversible');
    // Canonical short form, and the spelling the board's REF_RE links on.
    expect(written).toContain('(session:c03bd8ce)');
  });

  it('REFUSES a malformed session id, and writes nothing at all', () => {
    expect(() =>
      call('brief.append', {
        project: agentCwd,
        section: 'Recently',
        line: 'round two landed',
        sessionId: '6a-round2',
        result: { commit: 'abc1234' },
      }),
    ).toThrow(/not a session id/);
    expect(fs.existsSync(path.join(agentCwd, '.workspacer', 'brief.md'))).toBe(false);
  });

  it('REFUSES a result with no significance sentence — judgement is the manager’s job', () => {
    expect(() =>
      call('brief.append', {
        project: agentCwd,
        section: 'Recently',
        line: '   ',
        sessionId: LIVE,
        result: { commit: 'abc1234', filesChanged: ['a.ts'] },
      }),
    ).toThrow(/one-sentence significance/);
    expect(fs.existsSync(path.join(agentCwd, '.workspacer', 'brief.md'))).toBe(false);
  });

  it('still confines the project directory — the new params widen nothing', () => {
    expect(() =>
      call('brief.append', { project: '/etc', section: 'Now', line: 'x', sessionId: LIVE }),
    ).toThrow(/outside the allowed workspace/);
  });
});

describe('brief.check — flag a stale Now line, never touch the file', () => {
  const LIVE = 'c03bd8ce-1f4a-4b2c-9d3e-0123456789ab';
  const DEAD = 'deadbeef-1f4a-4b2c-9d3e-0123456789ab';
  let agentCwd: string;
  let briefPath: string;

  beforeEach(() => {
    agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-briefchk-')));
    fs.mkdirSync(path.join(agentCwd, '.workspacer'));
    briefPath = path.join(agentCwd, '.workspacer', 'brief.md');
    fs.writeFileSync(
      briefPath,
      '## Now\n' +
        '- dispatched the parser fix — session:c03bd8ce\n' +
        '- dispatched the lexer rewrite — session:deadbeef\n',
    );
    getAllSnapshots.mockReturnValue([
      { cwd: agentCwd, sessionId: LIVE, mode: 'running' },
      // Finished counts as GONE — that is the case that strands a Now line.
      { cwd: agentCwd, sessionId: DEAD, mode: 'stopped' },
    ] as never);
  });

  it('flags the dead reference and leaves the live one alone', () => {
    const report = call('brief.check', { project: agentCwd }) as {
      findings: { reason: string; refs: string[] }[];
      entriesChecked: number;
      entriesLive: number;
    };
    expect(report.entriesChecked).toBe(2);
    expect(report.entriesLive).toBe(1);
    expect(report.findings).toHaveLength(1);
    expect(report.findings[0].reason).toBe('stale');
    expect(report.findings[0].refs).toEqual(['deadbeef']);
  });

  it('NEVER writes — the brief is byte-identical afterwards', () => {
    const before = fs.readFileSync(briefPath, 'utf-8');
    const entriesBefore = fs.readdirSync(path.dirname(briefPath)).sort();
    call('brief.check', { project: agentCwd });
    expect(fs.readFileSync(briefPath, 'utf-8')).toBe(before);
    expect(fs.readdirSync(path.dirname(briefPath)).sort()).toEqual(entriesBefore);
  });

  it('answers for a project that has no brief yet, rather than throwing', () => {
    fs.rmSync(briefPath);
    const report = call('brief.check', { project: agentCwd }) as { entriesChecked: number };
    expect(report.entriesChecked).toBe(0);
  });

  it('is confined to the workspace roots, exactly like its writing siblings', () => {
    expect(() => call('brief.check', { project: '/etc' })).toThrow(/outside the allowed workspace/);
  });
});
