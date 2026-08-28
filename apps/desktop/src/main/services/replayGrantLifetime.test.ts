/**
 * A GRANT IS A FACT ABOUT THE PRESENT, and the replay worktree outlived the one
 * that created it.
 *
 * `replay.open` is confined — assertPathAllowed(cwd, workspaceRoots()), the same
 * guard git.* gets, because it cuts a git worktree from the repository at that
 * cwd. `replay.read` is not, and capspec says why: "the path is a repo-relative
 * coordinate inside a worktree the replay service itself created and keyed by
 * sessionId; containment is structural". The containment was real. What was not
 * real was the grant it stood on:
 *
 *   1. while a session is live, replay.open { cwd: <repo>, sessionId: "S" } is
 *      allowed and cuts a worktree;
 *   2. the session stops. snapshotGrantsFsRoot now refuses it, so <repo> leaves
 *      workspaceRoots(): fs.read on <repo> is refused, and a fresh
 *      replay.open { cwd: <repo> } is refused;
 *   3. replay.read { sessionId: "S" } went on returning that repository's bytes.
 *
 * `sessionId` is not an ownership token. It is caller-chosen at open, the entries
 * map is process-global, replay.* sits outside the bus's per-plugin fsRoots
 * scoping (policy.go names fs.read/fs.write/search.project, not replay.*), and
 * ids are handed out by agents.list and sessions.snapshots — both classified
 * inert, both labelled non-sensitive. So a token with replay.read and NO
 * filesystem roots at all could drink from a channel a different call opened.
 *
 * Every guard here is the shipping one: the real timelineReplayService, the real
 * pathConfinement, the real registered handlers. Only claudeSessionStore is
 * stubbed, and it is stubbed with the two states claudemon actually produces.
 */
import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest';
import { gatedIt, itRanEveryGatedTest, HAS_GIT } from '../../../tests/support/sweepTally';
import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';
import { execFileSync } from 'child_process';

const registered = new Map<string, (params: unknown) => unknown>();
vi.mock('./hubClient', () => ({
  registerCapability: (method: string, handler: (params: unknown) => unknown) => {
    registered.set(method, handler);
  },
}));
// Delegation OFF, for one reason only: it makes `fs.read` register in-process
// so it can be the CONTROL below. replay.* goes through registerCapability and
// is registered either way, so nothing under test changes. The brain's fs.read
// refuses the identical path — both copies are held to
// contracts/path-containment-cases.json.
vi.mock('./brainDelegation', () => ({ DELEGATE_CATALOG_TO_BRAIN: false }));

/** The snapshot set workspaceRoots() reads. Flipped from live to stopped below. */
const snapshots: Array<Record<string, unknown>> = [];
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    getAllSnapshots: () => snapshots,
    getSnapshot: vi.fn(),
    notePermissionMode: vi.fn(),
    clearPendingQuestions: vi.fn(),
  },
}));

const cfgDir = vi.hoisted(() => {
  const nodeFs = require('fs') as typeof import('fs');
  const nodeOs = require('os') as typeof import('os');
  const nodePath = require('path') as typeof import('path');
  return nodeFs.realpathSync(nodeFs.mkdtempSync(nodePath.join(nodeOs.tmpdir(), 'wks-replay-cfg-')));
});
vi.mock('./configService', () => ({
  configService: { getConfig: () => ({}), reloadConfig: vi.fn(), getConfigPath: vi.fn() },
  getConfigDir: () => cfgDir,
}));

// Everything else hubCapabilities pulls in at module scope. None of it is under
// test here; the replay service deliberately is NOT mocked.
vi.mock('./claudemonSessionClient', () => ({ claudemonSessionClient: {} }));
vi.mock('./managedSpawn', () => ({ spawnManagedAgent: vi.fn() }));
vi.mock('./claudeSpawn', () => ({ spawnClaudeAgent: vi.fn() }));
vi.mock('./agentProviders', () => ({
  checkAllProviders: vi.fn(),
  checkAllProvidersCached: vi.fn(),
  resolveAgentBinary: vi.fn(),
}));
vi.mock('./agentHandoff', () => ({ agentHandoffBrief: vi.fn() }));
vi.mock('electron', () => ({
  Notification: Object.assign(
    vi.fn(function (this: Record<string, unknown>) {
      this.show = vi.fn();
      this.on = vi.fn();
    }),
    { isSupported: () => false },
  ),
  shell: { openExternal: vi.fn() },
}));
vi.mock('./claudeProfiles', () => ({ claudeProfiles: {} }));
vi.mock('../lib/appIcon', () => ({ appIconPath: () => undefined }));
vi.mock('./claudeModels', () => ({ listClaudeModels: vi.fn(() => []) }));
vi.mock('./libraryService', () => ({ libraryService: {} }));
vi.mock('./agentNotifier', () => ({ agentNotifier: { postInApp: vi.fn(), focusAgent: vi.fn() } }));
vi.mock('./sessionService', () => ({ sessionService: {} }));
vi.mock('./sessionHistory', () => ({ sessionHistory: {} }));
vi.mock('./layoutService', () => ({ layoutService: {} }));
vi.mock('./claudeSessionList', () => ({ listClaudeSessionsForDir: vi.fn() }));
vi.mock('./recentSessions', () => ({ listRecentSessions: vi.fn() }));
vi.mock('./fileService', () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
  listDir: vi.fn(),
}));
vi.mock('./imagePreview', () => ({ readImagePreview: vi.fn() }));
vi.mock('./fileWatchService', () => ({ startWatch: vi.fn(), stopWatch: vi.fn() }));
vi.mock('./searchService', () => ({ searchProject: vi.fn() }));
vi.mock('./gitService', () => ({
  status: vi.fn(),
  log: vi.fn(),
  workRoot: vi.fn(),
  diff: vi.fn(),
  numstat: vi.fn(),
  stage: vi.fn(),
  unstage: vi.fn(),
  commit: vi.fn(),
  push: vi.fn(),
}));
vi.mock('./terminalShare', () => ({}));
vi.mock('../lib/workspacerHome', () => ({ ensureSupervisorHome: vi.fn() }));

const { registerHubCapabilities } = await import('./hubCapabilities');

async function attempt(method: string, params: unknown): Promise<string> {
  const handler = registered.get(method);
  if (!handler) throw new Error(`capability not registered: ${method}`);
  try {
    await handler(params);
    return '';
  } catch (err) {
    return (err as Error).message;
  }
}
async function read(sessionId: string, p: string): Promise<{ content?: string; err?: string }> {
  const handler = registered.get('replay.read');
  if (!handler) throw new Error('replay.read not registered');
  try {
    const r = (await handler({ sessionId, path: p })) as { content?: string };
    return { content: r?.content };
  } catch (err) {
    return { err: (err as Error).message };
  }
}

let repo: string;
const SECRET = 'REPLAY_SECRET_BYTES\n';

beforeAll(() => {
  registerHubCapabilities();
  if (!HAS_GIT) return;
  repo = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-replay-repo-')));
  const git = (...args: string[]): void => {
    execFileSync('git', args, {
      cwd: repo,
      stdio: 'ignore',
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: 't',
        GIT_AUTHOR_EMAIL: 't@t',
        GIT_COMMITTER_NAME: 't',
        GIT_COMMITTER_EMAIL: 't@t',
      },
    });
  };
  git('init', '-q');
  fs.writeFileSync(path.join(repo, 'secret.txt'), SECRET);
  git('add', '-A');
  git('commit', '-qm', 'seed');
});

afterAll(() => {
  if (repo) fs.rmSync(repo, { recursive: true, force: true });
  fs.rmSync(cfgDir, { recursive: true, force: true });
  fs.rmSync(path.join(os.tmpdir(), 'workspacer-replay', 'grant-lifetime'), {
    recursive: true,
    force: true,
  });
});

const gate = { ran: 0 };
const itGit = gatedIt(HAS_GIT, gate);

describe('replay.read — the grant that authorized the open must still hold', () => {
  itGit('stops serving the repository the moment the session that granted it stops', async () => {
    const sessionId = 'grant-lifetime';

    // ── While the session is LIVE ────────────────────────────────────────
    snapshots.length = 0;
    snapshots.push({ cwd: repo, mode: 'input' });

    expect(await attempt('replay.open', { cwd: repo, sessionId })).toBe('');
    const live = await read(sessionId, 'secret.txt');
    expect(live.err, 'replay.read must work while the grant holds').toBeUndefined();
    expect(live.content).toBe(SECRET);

    // ── The session STOPS ────────────────────────────────────────────────
    // The exact shape claudemon produces; snapshotGrantsFsRoot refuses it, so
    // workspaceRoots() no longer contains the repo.
    snapshots.length = 0;
    snapshots.push({ cwd: repo, mode: 'stopped' });

    // Both controls, in the same run: if either of these still passed, the
    // assertion below would be about a root set that never narrowed.
    expect(
      await attempt('fs.read', { path: path.join(repo, 'secret.txt') }),
      'the control: fs.read on the same directory must now be refused',
    ).toMatch(/outside the allowed workspace/);
    expect(
      await attempt('replay.open', { cwd: repo, sessionId: 'second' }),
      'the control: a FRESH replay.open on the same directory must now be refused',
    ).toMatch(/outside the allowed workspace/);

    // ── The channel opened under the revoked grant ───────────────────────
    const after = await read(sessionId, 'secret.txt');
    expect(
      after.content,
      'replay.read returned bytes from a repository fs.read and replay.open both refuse — the worktree outlived the grant that authorized it',
    ).toBeUndefined();
    expect(after.err).toMatch(/outside the allowed workspace/);

    // …and so do its siblings, which read and WRITE in the same worktree.
    expect(await attempt('replay.diff', { sessionId })).toMatch(/outside the allowed workspace/);
    expect(await attempt('replay.seek', { sessionId, ops: [] })).toMatch(
      /outside the allowed workspace/,
    );

    // ── THE FLOOR ────────────────────────────────────────────────────────
    // Re-grant and read again. Without this, a replay.read that throws
    // unconditionally — or a service that lost the entry entirely — passes every
    // assertion above while the feature is simply broken.
    snapshots.length = 0;
    snapshots.push({ cwd: repo, mode: 'input' });
    const regranted = await read(sessionId, 'secret.txt');
    expect(regranted.err, 'the re-granted read must work again').toBeUndefined();
    expect(regranted.content).toBe(SECRET);
  });

  // An id nobody opened must not be answered differently from one that is
  // simply out of roots: the service owns that message, and answering it in the
  // guard would make the bus an existence oracle for other callers' ids.
  it('leaves an unknown session id to the service rather than the guard', async () => {
    snapshots.length = 0;
    const msg = await attempt('replay.read', { sessionId: 'never-opened', path: 'x' });
    expect(msg).toMatch(/replay not open for this session/);
  });
});

itRanEveryGatedTest(gate, 'git', 1);
