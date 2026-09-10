/** Real HTTP MCP auth → Go bus → desktop hubClient → agents.spawn → disk.
 * Only provider launch is mocked. Electron is a headless runtime shim; no
 * production capability, identity, config, session or history store is mocked.
 */
import { afterAll, beforeAll, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { spawn, execFileSync, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { once, on } from 'node:events';
import { createInterface } from 'node:readline';
import WebSocket from 'ws';
import type { DispatchTask } from '../../src/main/shared/dispatchHistory';
import type { RequestIntent } from '../../src/main/shared/managerRequests';

vi.mock('electron', () => ({
  app: {
    isPackaged: false,
    getPath: () => process.env.HOME,
    getAppPath: () => process.cwd(),
  },
  BrowserWindow: class {
    static getAllWindows() {
      return [];
    }
  },
  Notification: class {
    static isSupported() {
      return false;
    }
  },
  shell: {},
  nativeImage: {},
}));
const launch = vi.hoisted(() => vi.fn());
vi.mock('../../src/main/services/managedSpawn', () => ({ spawnManagedAgent: launch }));
vi.mock('../../src/main/services/claudeSpawn', () => ({
  spawnClaudeAgent: () => {
    throw new Error('Unexpected Claude launch');
  },
}));

// Workspacer worktrees may auto-link dependencies into the primary checkout.
// Refuse that layout before writing any build cache or fixture state there.
if (fs.realpathSync('node_modules') !== path.resolve('node_modules')) {
  throw new Error('test:dispatch-chain requires a private desktop node_modules directory');
}
const cache = path.resolve('node_modules/.cache/dispatch-chain');
fs.mkdirSync(cache, { recursive: true });
const scratch = fs.mkdtempSync(path.join(cache, 'run-'));
const configRoot = path.join(scratch, 'config');
const configDir = path.join(configRoot, 'workspacer');
fs.mkdirSync(configDir, { recursive: true });
fs.mkdirSync(path.join(scratch, 'home'));
fs.writeFileSync(path.join(configDir, 'remote-token'), 'dispatch-chain-synthetic-host', {
  mode: 0o600,
});
fs.writeFileSync(path.join(configDir, 'config.yaml'), 'claude:\n  skipPermissionsDefault: false\n');
const project = path.join(scratch, 'project');
fs.mkdirSync(project);
execFileSync('git', ['init', '-q', project]);
execFileSync('git', [
  '-C',
  project,
  '-c',
  'user.name=Fixture',
  '-c',
  'user.email=fixture@example.test',
  'commit',
  '--allow-empty',
  '-qm',
  'fixture',
]);
const fixtureEnv = {
  // Deliberate allowlist: no inherited user tokens, provider credentials, or
  // remote-server settings can enter the subprocess.
  PATH: process.env.PATH,
  HOME: path.join(scratch, 'home'),
  USERPROFILE: path.join(scratch, 'home'),
  XDG_CONFIG_HOME: configRoot,
  APPDATA: configRoot,
  TMPDIR: scratch,
  GOCACHE: path.join(cache, 'go-build'),
  GOMODCACHE: path.join(cache, 'go-mod'),
  WKS_DISPATCH_CHAIN_FIXTURE: '1',
};
let child: ChildProcessWithoutNullStreams | undefined;
let childExit: Promise<unknown> | undefined;
let busURL: string;
let facadeURL: string;
let credentials: Array<{ label: string; token: string }>;
let hub: typeof import('../../src/main/services/hubClient') | undefined;
let sessions: typeof import('../../src/main/services/claudeSessionStore').claudeSessionStore;
let history: typeof import('../../src/main/services/dispatchHistoryStore').dispatchHistoryStore;
let validate: ReturnType<typeof vi.spyOn>;
let sequence = 0;

const tokenFor = (label: string) => {
  const rec = credentials.find((rec) => rec.label === label);
  if (!rec) throw new Error('Missing synthetic fixture credential');
  return rec.token;
};
const persisted = (): DispatchTask[] =>
  JSON.parse(fs.readFileSync(path.join(configDir, 'dispatch-history.json'), 'utf8')).tasks;

beforeAll(async () => {
  const binary = path.join(scratch, 'mcp-fixture');
  const build = spawn('go', ['test', '-c', '-o', binary, './cmd/mcp'], {
    cwd: path.resolve('../../services/hub'),
    env: fixtureEnv,
    stdio: 'pipe',
    timeout: 120_000,
  });
  let buildErrors = '';
  build.stderr.on('data', (data) => {
    buildErrors += data;
  });
  const [code] = await once(build, 'exit');
  if (code !== 0) throw new Error(`Go fixture build failed: ${buildErrors}`);
  child = spawn(binary, ['-test.run=^TestDesktopDispatchChainFixture$', '-test.timeout=300s'], {
    cwd: path.resolve('../../services/hub'),
    env: fixtureEnv,
    stdio: 'pipe',
  });
  childExit = once(child, 'exit');
  // Do not echo subprocess logs: even synthetic bearer material stays private.
  child.stderr.resume();
  const lines = createInterface({ input: child.stdout });
  const ready = await Promise.race([
    once(lines, 'line').then(([line]) => JSON.parse(line)),
    childExit.then(() => {
      throw new Error('Go fixture exited before readiness');
    }),
  ]);
  lines.close();
  ({ busURL, facadeURL } = ready);
  credentials = JSON.parse(fs.readFileSync(path.join(configDir, 'tokens.json'), 'utf8'));

  vi.stubEnv('HOME', fixtureEnv.HOME);
  vi.stubEnv('USERPROFILE', fixtureEnv.HOME);
  vi.stubEnv('XDG_CONFIG_HOME', configRoot);
  vi.stubEnv('APPDATA', configRoot);
  vi.stubEnv('WORKSPACER_REMOTE_SHARE', '');
  vi.stubEnv('WORKSPACER_PORT_OFFSET', String(Number(new URL(busURL).port) - 7895));
  // Import only after the scratch paths and allocated port exist. These are
  // consumed by real configService/hubDaemon/daemonUtils, not stub accessors.
  hub = await import('../../src/main/services/hubClient');
  const daemon = await import('../../src/main/services/hubDaemon');
  expect(daemon.hubBusUrl()).toBe(busURL);
  expect(daemon.getHubToken() === 'dispatch-chain-synthetic-host').toBe(true);
  const config = await import('../../src/main/services/configService');
  expect(config.getConfigDir()).toBe(configDir);
  const alias = path.join(scratch, 'project-alias');
  fs.symlinkSync(project, alias, 'dir');
  config.configService.saveConfig({ projects: { [alias]: { delivery: 'local' } } });
  ({ claudeSessionStore: sessions } = await import('../../src/main/services/claudeSessionStore'));
  ({ dispatchHistoryStore: history } =
    await import('../../src/main/services/dispatchHistoryStore'));
  validate = vi.spyOn(history, 'validate'); // Observation only: real validation still executes.
  for (const sessionId of ['manager-other', 'manager-current']) {
    // Distinct observed start times make manager-current the newest local manager.
    await new Promise((resolve) => setTimeout(resolve, 2));
    sessions.setSpawnMeta(sessionId, { isWakeTarget: true, provider: 'codex', label: sessionId });
    sessions.ensureManagedSession(sessionId, project);
  }
  expect(
    sessions
      .getAllSnapshots()
      .filter((s) => s.isWakeTarget && s.status !== 'ended' && !s.hub)
      .sort((a, b) => b.startedAt - a.startedAt)[0].sessionId,
  ).toBe('manager-current');
  launch.mockImplementation(async (opts) => {
    const id = `synthetic-worker-${++sequence}`;
    if (opts.firstMessage) {
      sessions.setSpawnMeta(id, {
        provider: 'codex',
        parentSessionId: opts.parentSessionId,
        resultSchema: opts.resultSchema,
      });
      sessions.ensureManagedSession(id, opts.cwd);
    }
    return id;
  });
  const { registerHubCapabilities } = await import('../../src/main/services/hubCapabilities');
  registerHubCapabilities();
  // Catalog delegation is production-default ON; register config.get using
  // the real service because this fixture intentionally has no brain process.
  hub.registerCapability('config.get', () => config.configService.getConfig());
  hub.startHubClient();
  await vi.waitFor(
    async () => {
      expect(hub!.isHubConnected()).toBe(true);
      const rows = await hub!.callHub<Array<{ sessionId: string }>>('agents.list');
      expect(rows.some((row) => row.sessionId === 'manager-current')).toBe(true);
      const health = (await fetch(`${facadeURL}/health`).then((r) => r.json())) as {
        hubConnected: boolean;
      };
      expect(health.hubConnected).toBe(true);
    },
    { timeout: 10_000 },
  );
});

afterAll(async () => {
  hub?.stopHubClient();
  history?.flush();
  child?.stdin.end();
  if (childExit) await childExit;
  vi.unstubAllEnvs();
  fs.rmSync(scratch, { recursive: true, force: true });
});

const mcpSessions = new Map<string, string>();
function decodeMcp(body: string, contentType: string | null) {
  return contentType?.includes('text/event-stream')
    ? JSON.parse(
        body
          .split('\n')
          .find((line) => line.startsWith('data: '))!
          .slice(6),
      )
    : JSON.parse(body);
}
async function mcpPost(label: string, message: Record<string, unknown>) {
  return fetch(`${facadeURL}/mcp`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${tokenFor(label)}`,
      'Content-Type': 'application/json',
      Accept: 'application/json, text/event-stream',
      ...(mcpSessions.has(label) ? { 'Mcp-Session-Id': mcpSessions.get(label)! } : {}),
    },
    body: JSON.stringify({ jsonrpc: '2.0', ...message }),
    signal: AbortSignal.timeout(15_000),
  });
}
async function mcpTool(label: string, name: string, args: Record<string, unknown>) {
  if (!mcpSessions.has(label)) {
    const init = await mcpPost(label, {
      id: ++sequence,
      method: 'initialize',
      params: {
        protocolVersion: '2025-03-26',
        capabilities: {},
        clientInfo: { name: 'dispatch-chain-fixture', version: '1' },
      },
    });
    expect(init.status).toBe(200);
    const envelope = decodeMcp(await init.text(), init.headers.get('content-type'));
    expect(envelope.error).toBeUndefined();
    const session = init.headers.get('mcp-session-id');
    expect(session).toEqual(expect.any(String));
    mcpSessions.set(label, session!);
    const initialized = await mcpPost(label, { method: 'notifications/initialized' });
    expect(initialized.status).toBe(202);
    await initialized.text();
  }
  const response = await mcpPost(label, {
    id: ++sequence,
    method: 'tools/call',
    params: {
      name,
      arguments: args,
    },
  });
  expect(response.status).toBe(200);
  const envelope = decodeMcp(await response.text(), response.headers.get('content-type'));
  expect(envelope.error).toBeUndefined();
  const text = envelope.result.content.map((c: { text: string }) => c.text).join('');
  return {
    isError: envelope.result.isError === true,
    text,
    value: envelope.result.isError ? undefined : JSON.parse(text),
  };
}

const mcpSpawn = (label: string, args: Record<string, unknown>) =>
  mcpTool(label, 'spawn_agent', {
    cwd: project,
    provider: 'codex',
    model: 'gpt-5',
    skipPermissions: false,
    ...args,
  });

async function busSpawn(
  token: string,
  params: Record<string, unknown>,
  federated = false,
  method = 'agents.spawn',
) {
  const socket = new WebSocket(
    `${busURL}?token=${encodeURIComponent(token)}${federated ? '&peer=1' : ''}`,
  );
  try {
    await once(socket, 'open');
    socket.send(JSON.stringify({ op: 'call', id: 'fixture-spawn', method, params }));
    for await (const [raw] of on(socket, 'message', { signal: AbortSignal.timeout(10_000) })) {
      const frame = JSON.parse(raw.toString());
      if (frame.id === 'fixture-spawn') return frame;
    }
  } finally {
    socket.close();
  }
}

it('rejects an unknown HTTP credential before launch', async () => {
  const before = launch.mock.calls.length;
  const response = await fetch(`${facadeURL}/mcp`, {
    method: 'POST',
    headers: { Authorization: 'Bearer dispatch-chain-invalid', 'Content-Type': 'application/json' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: {
        name: 'spawn_agent',
        arguments: { cwd: project, parentSessionId: 'manager-current' },
      },
    }),
    signal: AbortSignal.timeout(5_000),
  });
  expect(response.status).toBe(401);
  await response.text();
  expect(launch).toHaveBeenCalledTimes(before);
});

it('derives current manager from the HTTP credential and persists a linked task continuation', async () => {
  const first = await mcpSpawn('session:manager-current', {
    parentSessionId: 'manager-current',
    stage: 'implement',
  });
  expect(first.isError).toBe(false);
  expect(first.value).toMatchObject({ taskId: expect.any(String), dispatchId: expect.any(String) });
  const second = await mcpSpawn('session:manager-current', {
    parentSessionId: 'manager-current',
    taskId: first.value.taskId,
    afterDispatchId: first.value.dispatchId,
    stage: 'review',
  });
  expect(second.isError).toBe(false);
  expect(second.value.taskId).toBe(first.value.taskId);
  expect(second.value.dispatchId).not.toBe(first.value.dispatchId);
  expect(launch).toHaveBeenLastCalledWith(
    expect.objectContaining({
      cwd: project,
      parentSessionId: 'manager-current',
      provider: 'codex',
    }),
  );
  const tasks = persisted();
  expect(tasks).toHaveLength(1);
  expect(tasks[0]).toMatchObject({
    taskId: first.value.taskId,
    ownerSessionId: 'manager-current',
    projectCwd: project,
  });
  expect(tasks[0].attempts).toMatchObject([
    { dispatchId: first.value.dispatchId, sessionId: first.value.sessionId, stage: 'implement' },
    {
      dispatchId: second.value.dispatchId,
      sessionId: second.value.sessionId,
      stage: 'review',
      afterDispatchId: first.value.dispatchId,
    },
  ]);
  expect(fs.statSync(path.join(configDir, 'dispatch-history.json')).mode & 0o777).toBe(0o600);
  const { DispatchHistoryStore } = await import('../../src/main/services/dispatchHistoryStore');
  const reloaded = new DispatchHistoryStore(() => path.join(configDir, 'dispatch-history.json'));
  expect(reloaded.list()[0].attempts).toHaveLength(2);
  expect(reloaded.list()[0].attempts.every((attempt) => attempt.stale && !attempt.live)).toBe(true);
});

it('refuses cross-owner explicit links before launch, including a forged parent', async () => {
  const task = persisted()[0];
  for (const parentSessionId of ['manager-other', 'manager-current']) {
    const before = launch.mock.calls.length;
    const result = await mcpSpawn('session:manager-other', {
      parentSessionId,
      taskId: task.taskId,
      afterDispatchId: task.attempts[0].dispatchId,
      stage: 'fix',
    });
    expect(result.isError).toBe(true);
    expect(result.text).toMatch(/another manager|live local manager/);
    expect(launch).toHaveBeenCalledTimes(before);
  }
  expect(persisted()[0].attempts).toHaveLength(2);
});

it('allows optional stage with unattributed facade credentials without recording history', async () => {
  const before = launch.mock.calls.length;
  const result = await mcpSpawn('pairing', { parentSessionId: 'manager-current', stage: 'review' });
  expect(result.isError).toBe(false);
  expect(result.value.sessionId).toEqual(expect.any(String));
  expect(result.value.taskId).toBeUndefined();
  expect(result.value.dispatchId).toBeUndefined();
  expect(launch).toHaveBeenCalledTimes(before + 1);
  expect(persisted()).toHaveLength(1);
  expect(persisted()[0].attempts).toHaveLength(2);
});

it('keeps an unowned host retry launch valid without borrowing the source task', async () => {
  const task = persisted()[0];
  const before = launch.mock.calls.length;
  const result = await busSpawn('dispatch-chain-synthetic-host', {
    cwd: project,
    provider: 'codex',
    parentSessionId: 'manager-current',
    stage: 'fix',
    retrySourceSessionId: task.attempts[0].sessionId,
  });
  expect(result.op).toBe('result');
  expect(result.result.sessionId).toEqual(expect.any(String));
  expect(result.result.taskId).toBeUndefined();
  expect(result.result.dispatchId).toBeUndefined();
  expect(validate).toHaveBeenLastCalledWith(
    expect.objectContaining({
      owner: null,
      retrySourceSessionId: task.attempts[0].sessionId,
      stage: 'fix',
    }),
  );
  expect(launch).toHaveBeenCalledTimes(before + 1);
  expect(persisted()).toHaveLength(1);
  expect(persisted()[0].attempts).toHaveLength(2);
});

it.each(['scoped', 'plugin', 'federated'] as const)(
  '%s strips owner/retry stamps, preserves optional stage, and refuses explicit links',
  async (kind) => {
    const token =
      kind === 'scoped'
        ? tokenFor('session:manager-current')
        : kind === 'plugin'
          ? 'dispatch-chain-synthetic-plugin'
          : 'dispatch-chain-synthetic-host';
    const task = persisted()[0];
    const params = {
      cwd: project,
      provider: 'codex',
      parentSessionId: 'manager-current',
      stage: 'fix',
      dispatchOwnerSessionId: 'manager-current',
      retrySourceSessionId: task.attempts[0].sessionId,
    };
    const before = launch.mock.calls.length;
    const accepted = await busSpawn(token, params, kind === 'federated');
    expect(accepted.op).toBe('result');
    expect(accepted.result.sessionId).toEqual(expect.any(String));
    expect(accepted.result.taskId).toBeUndefined();
    expect(accepted.result.dispatchId).toBeUndefined();
    expect(validate).toHaveBeenLastCalledWith(
      expect.objectContaining({ owner: null, retrySourceSessionId: undefined, stage: 'fix' }),
    );
    expect(launch).toHaveBeenCalledTimes(before + 1);
    const refused = await busSpawn(
      token,
      { ...params, taskId: task.taskId, afterDispatchId: task.attempts[0].dispatchId },
      kind === 'federated',
    );
    expect(refused.op).toBe('error');
    expect(refused.error).toMatch(/live local manager/);
    expect(launch).toHaveBeenCalledTimes(before + 1);
    expect(persisted()).toHaveLength(1);
    expect(persisted()[0].attempts).toHaveLength(2);
  },
);

it('executes two selected policies through authenticated facade, desktop spawn, real wake validation and pinned history', async () => {
  const manager = 'session:manager-current';
  const call = async (name: string, args: Record<string, unknown> = {}) => {
    const r = await mcpTool(manager, name, args);
    expect(r.isError, r.text).toBe(false);
    return r.value;
  };
  const route = async (role: string) => {
    const d = await call('select_model', { role, cwd: project });
    expect(d.eligible).toBe(true);
    return {
      provider: d.provider,
      model: d.model,
      effort: d.effort,
      capability: d.capability,
      decisionId: d.decisionId,
    };
  };
  const { claudemonSessionClient } = await import('../../src/main/services/claudemonSessionClient');
  const { supervisorNudge } = await import('../../src/main/services/supervisorNudge');
  const delivery = vi.spyOn(claudemonSessionClient, 'message').mockResolvedValue({ ok: true });
  const finish = async (sessionId: string, result: unknown) => {
    const snapshot = sessions.getSnapshot(sessionId)!;
    const reply =
      'Actual fixture provider output\n```wks-result\n' + JSON.stringify(result) + '\n```';
    supervisorNudge.onFinished(
      {
        ...snapshot,
        resultSchema: undefined,
        status: 'active',
        ambientState: 'idle',
        conversation: [
          { role: 'user', content: 'fixture task' },
          { role: 'assistant', content: reply },
        ],
      },
      'manager-current',
      reply,
    );
    await vi.waitFor(
      () =>
        expect(
          history
            .task(
              history.list().find((t) => t.attempts.some((a) => a.sessionId === sessionId))!.taskId,
            )
            ?.workflow?.steps.find((s) => s.sessionId === sessionId)?.state,
        ).toBe('completed'),
      { timeout: 8000 },
    );
    expect(delivery).toHaveBeenLastCalledWith(
      'manager-current',
      expect.stringContaining('Fleet workflow'),
      // Handoff keeps each accepted finish's dedup identity with the send.
      [[sessionId, `${reply} 0  `]],
    );
  };
  try {
    let listed = await call('list_workflows');
    expect(listed.catalog.definitions).toHaveLength(2);
    const clone = await call('clone_workflow', {
      id: 'scout-implement-review',
      expectedRevision: 1,
      name: 'Pinned custom policy',
    });
    const id = clone.definition.id;
    expect(
      (
        await call('select_project_workflow', {
          cwd: project,
          workflowId: id,
          expectedRevision: listed.catalog.selectionRevision,
        })
      ).ok,
    ).toBe(true);
    const started = await call('start_workflow', { cwd: project, title: 'Pinned reviewed task' });
    const taskId = started.task.taskId;
    expect(started.task.workflow.steps.map((s: { state: string }) => s.state)).toEqual([
      'planned',
      'planned',
      'planned',
    ]);
    expect(
      (
        await call('decide_workflow_step', {
          cwd: project,
          taskId,
          stepId: 'scout',
          run: false,
          reason: 'No unresolved material risk',
        })
      ).task.workflow.steps[0].state,
    ).toBe('skipped');
    const edit = { ...clone.definition, name: 'Changed for later tasks' };
    expect(
      (await call('update_workflow', { id, expectedRevision: 1, definition: edit })).definition
        .revision,
    ).toBe(2);
    expect(
      (await call('update_workflow', { id, expectedRevision: 1, definition: edit })).code,
    ).toBe('conflict');
    // Template edits after task start must not change the worker prompt or result contract.
    const { libraryService } = await import('../../src/main/services/libraryService');
    const original = libraryService
      .list()
      .find((t) => t.id === 'ship-task' && t.scope === 'global')!;
    const source = fs.readFileSync(original.path, 'utf8');
    fs.writeFileSync(original.path, source.replace('SHIP TASK', 'ALTERED TEMPLATE'));
    const implementation = await mcpSpawn(manager, {
      parentSessionId: 'manager-current',
      taskId,
      workflowStepId: 'implement',
      stage: 'implement',
      role: 'implementer',
      ...(await route('implementer')),
      template: 'ship-task',
      templateParams: { task: 'Implement the fixture' },
      skipPermissions: true,
    });
    fs.writeFileSync(original.path, source);
    expect(implementation.isError, implementation.text).toBe(false);
    const opts = launch.mock.lastCall![0];
    expect(opts.firstMessage).toContain('SHIP TASK');
    expect(opts.firstMessage).toContain('approved local merge');
    expect(opts.firstMessage).not.toContain('ALTERED TEMPLATE');
    expect(opts.toolScope).toBe('view');
    expect(opts.skipPermissions).toBe(false);
    expect(opts.cwd).not.toBe(project);
    expect(opts.resultSchema.required).toContain('commit');
    const stillPinned = await call('next_workflow_step', { taskId, cwd: project });
    expect(stillPinned.task.workflow.definition.revision).toBe(1);
    expect(stillPinned.task.workflow.steps[1].state).toBe('dispatched');
    await finish(implementation.value.sessionId, {
      commit: 'fixture-commit',
      caveats: 'Example reported outcome; no success verdict inferred',
    });
    const reviewArgs = {
      parentSessionId: 'manager-current',
      taskId,
      workflowStepId: 'review',
      stage: 'review',
      role: 'reviewer',
      afterDispatchId: implementation.value.dispatchId,
      ...(await route('reviewer')),
      template: 'review-task',
      templateParams: { task: 'Check fixture criteria', handoff: 'fixture branch and checks' },
    };
    const reused = await mcpSpawn(manager, {
      ...reviewArgs,
      resumeSessionId: implementation.value.sessionId,
    });
    expect(reused.isError).toBe(true);
    listed = await call('list_workflows');
    expect(
      (
        await call('select_project_workflow', {
          cwd: project,
          workflowId: null,
          expectedRevision: listed.catalog.selectionRevision,
        })
      ).ok,
    ).toBe(true);
    expect((await call('disable_workflow', { id, expectedRevision: 2 })).ok).toBe(true);
    const review = await mcpSpawn(manager, reviewArgs);
    expect(review.isError, review.text).toBe(false);
    expect(review.value.sessionId).not.toBe(implementation.value.sessionId);
    expect(launch.mock.lastCall![0].toolScope).toBe('view');
    await finish(review.value.sessionId, {
      verdict: 'changes required',
      blocking: ['A reported blocker remains'],
    });
    const done = await call('next_workflow_step', { cwd: project, taskId });
    expect(done.task.workflow.steps[2].outcome.verdict).toBe('changes required');
    expect(done.instructions).toContain('NOT a passing verdict');
    listed = await call('list_workflows');
    expect(
      (
        await call('select_default_workflow', {
          workflowId: 'direct-implementation',
          expectedRevision: listed.catalog.selectionRevision,
        })
      ).ok,
    ).toBe(true);
    const direct = await call('start_workflow', { cwd: project, title: 'Explicit direct task' });
    expect(direct.instructions).toContain('review omitted by selected policy');
    const worker = await mcpSpawn(manager, {
      parentSessionId: 'manager-current',
      taskId: direct.task.taskId,
      workflowStepId: 'implement',
      stage: 'implement',
      role: 'implementer',
      ...(await route('implementer')),
      template: 'ship-task',
      templateParams: { task: 'Direct implementation fixture' },
    });
    expect(worker.isError, worker.text).toBe(false);
    await finish(worker.value.sessionId, { commit: 'direct-fixture' });
    expect(history.task(taskId)!.attempts.map((a) => a.stage)).toEqual(['implement', 'review']);
    expect(history.task(direct.task.taskId)!.attempts.map((a) => a.stage)).toEqual(['implement']);
    const foreign = await mcpTool('session:manager-other', 'next_workflow_step', {
      taskId,
      cwd: project,
    });
    expect(foreign.value.ok).toBe(false);
    const { DispatchHistoryStore } = await import('../../src/main/services/dispatchHistoryStore');
    const reopened = new DispatchHistoryStore(() => path.join(configDir, 'dispatch-history.json'));
    expect(reopened.task(taskId)!.workflow).toEqual(history.task(taskId)!.workflow);
    expect(
      (
        await call('next_workflow_step', {
          taskId: history.list().find((t) => !t.workflow)!.taskId,
          cwd: project,
        })
      ).ok,
    ).toBe(false);
  } finally {
    delivery.mockRestore();
  }
}, 60000);

it('confines workflow management to local host facade and rejects peer forwarding', async () => {
  for (const token of [tokenFor('session:manager-current'), 'dispatch-chain-synthetic-plugin']) {
    const frame = await busSpawn(
      token,
      { op: 'list', callerSessionId: 'manager-current' },
      false,
      'fleetWorkflows.request',
    );
    expect(frame.op).toBe('error');
  }
  const peer = await busSpawn(
    'dispatch-chain-synthetic-host',
    { op: 'list' },
    false,
    'hub:foreign/fleetWorkflows.request',
  );
  expect(peer.op).toBe('error');
  expect(peer.error).toContain('local desktop');
  const external = await mcpTool('pairing', 'list_workflows', {});
  expect(external.isError).toBe(true);
});

it('runs a custom research-only template, refuses missing inputs, and records invalid results and escalation honestly', async () => {
  const manager = 'session:manager-current';
  const call = async (name: string, args: Record<string, unknown> = {}) => {
    const r = await mcpTool(manager, name, args);
    expect(r.isError, r.text).toBe(false);
    return r.value;
  };
  fs.writeFileSync(
    path.join(configDir, 'library', 'workflow-research.md'),
    '---\ntitle: Research fixture\nkind: dispatch\nresultSchema:\n  type: object\n  required: [proven]\n  properties:\n    proven:\n      type: boolean\n---\n\n{{task}}\nResearch subject: {{subject}}\n',
  );
  const definition = {
    id: 'research-only',
    revision: 1,
    name: 'Research only',
    description: 'No implementation required',
    enabled: true,
    steps: [
      {
        id: 'inspect',
        label: 'Inspect source',
        kind: 'research',
        stage: 'other',
        role: 'diagnostician',
        when: 'always',
        template: 'workflow-research',
        instructions: 'Preserve this exact custom instruction.',
      },
    ],
  };
  expect((await call('create_workflow', { definition })).ok).toBe(true);
  const listed = await call('list_workflows');
  expect(
    (
      await call('select_project_workflow', {
        cwd: project,
        workflowId: definition.id,
        expectedRevision: listed.catalog.selectionRevision,
      })
    ).ok,
  ).toBe(true);
  const { claudemonSessionClient } = await import('../../src/main/services/claudemonSessionClient');
  const message = vi.spyOn(claudemonSessionClient, 'message').mockResolvedValue({ ok: true });
  const { supervisorNudge } = await import('../../src/main/services/supervisorNudge');
  try {
    for (const state of ['failed', 'blocked'] as const) {
      const task = (await call('start_workflow', { cwd: project, title: `Research ${state}` }))
        .task;
      const d = await call('select_model', { role: 'diagnostician', cwd: project });
      const params = {
        parentSessionId: 'manager-current',
        taskId: task.taskId,
        workflowStepId: 'inspect',
        stage: 'other',
        role: 'diagnostician',
        template: 'workflow-research',
        provider: d.provider,
        model: d.model,
        effort: d.effort,
        capability: d.capability,
        decisionId: d.decisionId,
      };
      const before = launch.mock.calls.length;
      const managerOverride = await mcpSpawn(manager, {
        ...params,
        manager: true,
        templateParams: { task: 'Read source', subject: 'parser' },
      });
      expect(managerOverride.isError).toBe(true);
      expect(launch.mock.calls).toHaveLength(before);
      const missing = await mcpSpawn(manager, {
        ...params,
        templateParams: { task: 'Read the source' },
      });
      expect(missing.isError).toBe(true);
      expect(launch.mock.calls).toHaveLength(before);
      const worker = await mcpSpawn(manager, {
        ...params,
        templateParams: { task: 'Read the source', subject: 'the parser' },
      });
      expect(worker.isError, worker.text).toBe(false);
      expect(launch.mock.lastCall![0]).toMatchObject({
        cwd: project,
        toolScope: 'view',
        routing: { role: 'diagnostician' },
      });
      expect(launch.mock.lastCall![0].firstMessage).toContain(
        'Preserve this exact custom instruction.',
      );
      const reply =
        state === 'failed'
          ? '```wks-result\n{"proven":"not a boolean"}\n```'
          : '```wks-escalation\n' +
            JSON.stringify({
              type: 'worker-escalation',
              status: 'blocked',
              reason: 'Need a decision',
              requiredAuthorityOrDecision: 'Choose scope',
              changed: false,
              nextAction: 'Clarify scope',
            }) +
            '\n```';
      supervisorNudge.onFinished(
        {
          ...sessions.getSnapshot(worker.value.sessionId)!,
          status: 'active',
          ambientState: 'idle',
          conversation: [
            { role: 'user', content: 'Read source' },
            { role: 'assistant', content: reply },
          ],
        },
        'manager-current',
        reply,
      );
      await vi.waitFor(
        () => expect(history.task(task.taskId)!.workflow!.steps[0].state).toBe(state),
        { timeout: 8000 },
      );
      expect(
        (await call('next_workflow_step', { taskId: task.taskId, cwd: project })).instructions,
      ).toContain('Do not launch another step');
      const retry = await mcpSpawn(manager, {
        ...params,
        templateParams: { task: 'Read the source', subject: 'the parser' },
      });
      expect(retry.isError).toBe(true);
    }
  } finally {
    message.mockRestore();
  }
});

it('records supplied task references through authenticated MCP and preserves human edits and review policy', async () => {
  const call = (name: string, args: Record<string, unknown>, label = 'session:manager-current') =>
    mcpTool(label, name, args);
  const started = await call('start_workflow', { cwd: project, title: 'Reference transport' });
  const task = started.value.task as DispatchTask;
  const taskId = task.taskId;
  const human = history.editByHostUser(
    {
      taskId,
      expectedTaskRevision: task.revision ?? 0,
      action: 'links',
      links: { tickets: [{ id: 'HUMAN-1' }] },
    },
    (id) => sessions.getSnapshot(id) ?? undefined,
    () => false,
  );
  expect(human.ok).toBe(true);
  const read = await call('get_task_references', { taskId, cwd: project });
  const args = {
    taskId,
    cwd: project,
    expectedTaskRevision: read.value.taskRevision,
    upsert: [{ kind: 'pullRequest', url: 'https://dev.azure.test/p/_git/r/pullrequest/9492' }],
  };
  const written = await call('update_task_references', args);
  expect(written.isError).toBe(false);
  expect(written.value).toMatchObject({
    ok: true,
    references: { tickets: [{ id: 'HUMAN-1' }], pullRequest: { url: args.upsert[0].url } },
  });
  expect(written.value.taskRevision).toBe(read.value.taskRevision + 1);
  expect(written.value.task.audit.at(-1).actor).toBe('manager');
  expect(written.value.task.workflow).toEqual(task.workflow);
  const repeated = await call('update_task_references', {
    ...args,
    expectedTaskRevision: written.value.taskRevision,
  });
  expect(repeated.value.taskRevision).toBe(written.value.taskRevision);
  const stale = await call('update_task_references', args);
  expect(stale.value).toMatchObject({
    ok: false,
    code: 'conflict',
    currentRevision: written.value.taskRevision,
    references: written.value.references,
  });
  for (const label of ['session:manager-other', 'pairing']) {
    const refused = await call('update_task_references', args, label);
    expect(refused.isError || refused.value?.ok === false).toBe(true);
  }
  expect((await call('get_task_references', { taskId, cwd: `${project}/wrong` })).value.ok).toBe(
    false,
  );
  expect(history.task(taskId)!.links).toEqual(written.value.references);
  for (const [token, federated] of [
    [tokenFor('session:manager-current'), false],
    ['dispatch-chain-synthetic-plugin', false],
    ['dispatch-chain-synthetic-host', true],
  ] as const) {
    const denied = await busSpawn(
      token,
      { ...args, op: 'setTaskReferences', callerSessionId: 'manager-current' },
      federated,
      'fleetWorkflows.request',
    );
    expect(denied.error).toBeTruthy();
  }
});

it('resolves authoritative multi-intent inbox requests through real authenticated MCP and transfers source ownership', async () => {
  const { managerRequests } = await import('../../src/main/services/managerRequestService');
  const inbox = managerRequests();
  const capture = inbox.prepare(
    'manager-current',
    'Fix https://business.visualstudio.com/Project/_git/Repo/pullrequest/9492 and document https://example.com/spec.',
  );
  if (!capture.available) throw new Error('Capture unavailable');
  const delivery = inbox.beginDelivery('manager-current', capture.requestId)!;
  inbox.finishDelivery(capture.requestId, delivery.deliveryId, 'unknown');
  const call = (name: string, args: Record<string, unknown>, label = 'session:manager-current') =>
    mcpTool(label, name, args);
  const fetched = await call('get_manager_request', { requestId: capture.requestId });
  expect(fetched.value.userContent).toEqual({
    text: 'Fix https://business.visualstudio.com/Project/_git/Repo/pullrequest/9492 and document https://example.com/spec.',
    trust: 'user',
  });
  expect(fetched.value.host.delivery).toBe('unknown');
  const intents: RequestIntent[] = ['Fix issue', 'Document behavior'].map((title, i) => ({
    key: `intent-${i}`,
    references:
      i === 0
        ? [
            {
              kind: 'pullRequest',
              number: '9492',
              url: 'https://business.visualstudio.com/Project/_git/Repo/pullrequest/9492',
            },
          ]
        : [{ kind: 'reference', label: 'Specification', url: 'https://example.com/spec' }],
    kind: 'create',
    cwd: project,
    title,
    provenance: 'explicit',
    reason: 'Independent user-requested work',
  }));
  const args = {
    requestId: capture.requestId,
    expectedRevision: fetched.value.host.revision,
    intents,
  };
  const before = launch.mock.calls.length;
  // list() projects a moving wallMs clock for live attempts. Compare the
  // complete persisted records instead, including links, lineage and revisions.
  const persistedTasks = () => history.requestTransaction((_requests, tasks) => tasks);
  const beforeTasks = persistedTasks();
  const invalid = await call('resolve_manager_request', {
    ...args,
    intents: [{ ...intents[0], references: [{ kind: 'pullRequest', number: '999' }] }, intents[1]],
  });
  expect(invalid.value).toMatchObject({
    ok: false,
    error: expect.stringMatching(/PR number must match/),
  });
  expect(persistedTasks()).toEqual(beforeTasks);
  expect(
    (await call('get_manager_request', { requestId: capture.requestId })).value.userContent,
  ).toEqual(fetched.value.userContent);
  const genericAsPr = await call('resolve_manager_request', {
    ...args,
    intents: [
      { ...intents[0], references: [{ kind: 'pullRequest', url: 'https://example.com/spec' }] },
      intents[1],
    ],
  });
  expect(genericAsPr.value).toMatchObject({
    ok: false,
    error: expect.stringMatching(/PR URL must identify/),
  });
  expect(persistedTasks()).toEqual(beforeTasks);
  expect(
    (await call('get_manager_request', { requestId: capture.requestId })).value.userContent,
  ).toEqual(fetched.value.userContent);
  const resolved = await call('resolve_manager_request', args);
  expect(resolved.value.ok).toBe(true);
  expect(resolved.value.tasks).toHaveLength(2);
  expect(resolved.value.tasks[0].links.pullRequest.number).toBe('9492');
  expect(resolved.value.tasks[1].links.references[0].label).toBe('Specification');
  expect(
    resolved.value.tasks.every(
      (t: DispatchTask) => t.sources?.[0].requestId === capture.requestId && !t.attempts.length,
    ),
  ).toBe(true);
  expect(launch.mock.calls).toHaveLength(before);
  const retry = await call('resolve_manager_request', args);
  expect(retry.value.tasks.map((t: DispatchTask) => t.taskId)).toEqual(
    resolved.value.tasks.map((t: DispatchTask) => t.taskId),
  );
  const foreign = await call(
    'get_manager_request',
    { requestId: capture.requestId },
    'session:manager-other',
  );
  expect(foreign.value.ok).toBe(false);
  const follow = inbox.prepare('manager-current', 'Publish nightly after the fixes.');
  if (!follow.available) throw new Error('Capture unavailable');
  const attempt = inbox.beginDelivery('manager-current', follow.requestId)!;
  inbox.finishDelivery(follow.requestId, attempt.deliveryId, 'accepted');
  const followup = await call('resolve_manager_request', {
    requestId: follow.requestId,
    expectedRevision: inbox.request('manager-current', follow.requestId).revision,
    intents: [
      {
        key: 'nightly',
        kind: 'followUp',
        cwd: project,
        title: 'Publish nightly',
        provenance: 'explicit',
        reason: 'Separate user-authorized delivery task',
        dependsOn: [resolved.value.tasks[0].taskId],
      },
    ],
  });
  expect(followup.value.ok).toBe(true);
  const next = await call('next_workflow_step', {
    cwd: project,
    taskId: followup.value.tasks[0].taskId,
  });
  expect(next.value.instructions).toContain('blocked');
  expect(launch.mock.calls).toHaveLength(before);
  history.adoptWorkflowTasks('manager-current', 'manager-other');
  expect((await call('get_manager_request', { requestId: capture.requestId })).value.ok).toBe(
    false,
  );
  const transferred = await call(
    'get_manager_request',
    { requestId: capture.requestId },
    'session:manager-other',
  );
  expect(transferred.value.host).toMatchObject({
    requestId: capture.requestId,
    sourceSessionId: 'manager-current',
    ownerSessionId: 'manager-other',
  });
  expect(transferred.value.userContent).toBeUndefined();
});

it('handles unknown paired replay idempotently and returns a local task result once', async () => {
  const binary = path.join(scratch, 'paired-brain-fixture');
  const build = spawn('go', ['test', '-c', '-o', binary, './cmd/brain'], {
    cwd: path.resolve('../../services/hub'),
    env: fixtureEnv,
    stdio: 'pipe',
    timeout: 120_000,
  });
  let errors = '';
  build.stderr.on('data', (data) => {
    errors += data;
  });
  build.stdout.resume();
  const [code] = await once(build, 'exit');
  if (code !== 0) throw new Error(errors);
  const peer = spawn(binary, ['-test.run=^TestPairedDispatchHostFixture$', '-test.timeout=120s'], {
    cwd: path.resolve('../../services/hub'),
    env: {
      ...fixtureEnv,
      WKS_PAIRED_CHAIN_FIXTURE: '1',
      WKS_HANDOFF_ORIGIN_BUS: busURL,
      WKS_HANDOFF_SOURCE: project,
      WKS_HANDOFF_ORIGIN_CONFIG: configDir,
    },
    stdio: 'pipe',
  });
  peer.stderr.resume();
  const lines = createInterface({ input: peer.stdout });
  const ready = await Promise.race([
    once(lines, 'line').then(([line]) => JSON.parse(line)),
    once(peer, 'exit').then(() => {
      throw new Error('Remote fixture exited before readiness');
    }),
  ]);
  lines.close();
  const { setRemoteServer, getRemoteServer } = await import('../../src/main/services/remoteServer');
  const { pairedWorkerConnection } = await import('../../src/main/services/pairedWorkerConnection');
  const { claudemonSessionClient } = await import('../../src/main/services/claudemonSessionClient');
  const delivery = vi.spyOn(claudemonSessionClient, 'message').mockResolvedValue({ ok: true });
  const pairedCalls = vi.spyOn(pairedWorkerConnection, 'call');
  try {
    setRemoteServer({ url: ready.oldURL, token: 'paired-fixture-operator', mode: 'workers' });
    const oldPeer = await mcpTool('session:manager-current', 'list_dispatch_targets', {});
    expect(oldPeer.value.targets.find((t: { name: string }) => t.name === 'paired').ready).toBe(
      false,
    );
    pairedWorkerConnection.stop();
    setRemoteServer({
      url: ready.url,
      token: 'paired-fixture-operator',
      mode: 'workers',
      displayName: 'Paired fixture workspace',
    });
    expect(getRemoteServer()).toBeNull();
    const discovery = await mcpTool('session:manager-current', 'list_dispatch_targets', {});
    expect(discovery.isError, discovery.text).toBe(false);
    expect(discovery.value.targets).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          name: 'paired',
          ready: true,
          cwds: expect.arrayContaining([expect.objectContaining({ path: ready.repo })]),
        }),
      ]),
    );
    const localLaunches = launch.mock.calls.length;
    await expect(
      pairedWorkerConnection.call('agents.dispatchPrepare', {
        remoteOrigin: { protocol: 2, dispatchId: 'failed-worktree-fixture-nonce' },
        cwd: ready.nonRepo,
        provider: 'claude',
        worktree: true,
      }),
    ).rejects.toThrow(/worktree allocation failed/);

    for (const override of [
      { remoteCwd: project },
      { provider: 'codex' },
      { parentSessionId: 'manager-other' },
    ]) {
      const rejected = await mcpSpawn('session:manager-current', {
        provider: 'claude',
        model: 'sonnet',
        executionTarget: 'paired',
        remoteCwd: ready.repo,
        parentSessionId: 'manager-current',
        message: 'Do not launch',
        ...override,
      });
      expect(rejected.isError, rejected.text).toBe(true);
    }
    const { PairedWorkerConnection } =
      await import('../../src/main/services/pairedWorkerConnection');
    const view = new PairedWorkerConnection(() => ({
      httpUrl: ready.url,
      busUrl: ready.url.replace('http:', 'ws:') + '/bus',
      token: 'paired-fixture-view',
    }));
    try {
      await expect(
        view.call('agents.spawn', { cwd: ready.repo, provider: 'claude', skipPermissions: true }),
      ).rejects.toThrow(/authoriz|scope/);
      await expect(
        view.call('agents.dispatchPrepare', { cwd: ready.repo, provider: 'claude' }),
      ).rejects.toThrow(/authoriz|scope/);
    } finally {
      view.stop();
    }

    const workflows = await mcpTool('session:manager-current', 'list_workflows', {});
    await mcpTool('session:manager-current', 'select_project_workflow', {
      cwd: project,
      workflowId: 'scout-implement-review',
      expectedRevision: workflows.value.catalog.selectionRevision,
    });
    const { managerRequests } = await import('../../src/main/services/managerRequestService');
    const inbox = managerRequests();
    const request = inbox.prepare(
      'manager-current',
      'Read the remote fixture and return the result contract.',
    );
    if (!request.available) throw new Error('Request capture unavailable');
    const send = inbox.beginDelivery('manager-current', request.requestId)!;
    inbox.finishDelivery(request.requestId, send.deliveryId, 'accepted');
    const resolved = await mcpTool('session:manager-current', 'resolve_manager_request', {
      requestId: request.requestId,
      expectedRevision: inbox.request('manager-current', request.requestId).revision,
      intents: [
        {
          key: 'paired-task',
          kind: 'create',
          cwd: project,
          title: 'Paired fixture',
          provenance: 'explicit',
          reason: 'User requested this remote task',
        },
      ],
    });
    expect(resolved.isError, resolved.text).toBe(false);
    const task = resolved.value.tasks[0] as DispatchTask;
    await mcpTool('session:manager-current', 'decide_workflow_step', {
      cwd: project,
      taskId: task.taskId,
      stepId: 'scout',
      run: false,
      reason: 'Fixture scope is established',
    });
    const route = await mcpTool('session:manager-current', 'select_dispatch_model', {
      cwd: ready.repo,
      provider: 'claude',
      role: 'implementer',
    });
    expect(route.isError, route.text).toBe(false);
    const result = await mcpSpawn('session:manager-current', {
      provider: 'claude',
      model: route.value.model,
      capability: route.value.capability,
      decisionId: route.value.decisionId,
      role: 'implementer',
      executionTarget: 'paired',
      remoteCwd: ready.repo,
      parentSessionId: 'manager-current',
      taskId: task.taskId,
      workflowStepId: 'implement',
      stage: 'implement',
      template: 'ship-task',
      templateParams: { task: 'Read the remote fixture and return the result contract.' },
      toolScope: 'view',
    });
    expect(result.isError, result.text).toBe(false);
    expect(launch.mock.calls.length).toBe(localLaunches);
    expect(result.value.sessionId).toMatch(/^paired:/);
    expect(result.value.executionCwd).not.toBe(project);
    expect(result.value.executionCwd).not.toBe(ready.repo);
    expect(fs.existsSync(path.join(result.value.executionCwd, '.git'))).toBe(true);
    const evidence = (await (await fetch(ready.control + '/evidence')).json()) as Array<{
      cwd: string;
      first_message: string;
    }>;
    expect(evidence).toHaveLength(1);
    expect(evidence[0].cwd).toBe(result.value.executionCwd);
    expect(evidence[0].first_message).toContain('Read the remote fixture');
    expect(JSON.stringify(evidence)).not.toContain('paired-fixture-operator');
    expect(JSON.stringify(evidence)).not.toContain('manager-current');
    expect(JSON.stringify(evidence)).not.toContain(task.taskId);
    expect(JSON.stringify(evidence)).not.toContain(request.requestId);
    expect(evidence[0]).not.toHaveProperty('workflowStepId');
    const { remoteDispatchRegistry } =
      await import('../../src/main/services/remoteDispatchRegistry');
    const record = remoteDispatchRegistry
      .list()
      .find((r) => r.localSessionId === result.value.sessionId)!;
    const reconnect = async (mode: string, target = record) => {
      await fetch(ready.control + '/control', {
        method: 'POST',
        body: JSON.stringify({ kind: mode }),
      });
      const from = pairedCalls.mock.calls.length;
      pairedWorkerConnection.stop();
      await pairedWorkerConnection.connect();
      let index = -1;
      await vi.waitFor(() => {
        index = pairedCalls.mock.calls.findIndex(
          ([method, params], i) =>
            i >= from &&
            method === 'agents.dispatchReplay' &&
            (params as { dispatchId?: string })?.dispatchId === target.dispatchId,
        );
        expect(index).toBeGreaterThanOrEqual(0);
      });
      // Await the actual onConnected RPC; its production response handler was
      // registered first, so negative assertions cannot pass before it runs.
      await Promise.resolve(pairedCalls.mock.results[index].value).catch(() => {});
    };
    const admitted = structuredClone(record);
    const beforeWakes = delivery.mock.calls.length;
    await reconnect('replay-error');
    expect(record).toEqual(admitted);
    await reconnect('replay-wrong-id');
    expect(record).toEqual(admitted);
    await reconnect('replay-unknown');
    expect(record.note).toContain('no longer has a record');
    expect(record.note).toContain('Do not repeat the spawn');
    expect(record).toEqual({ ...admitted, note: record.note });
    const visible = await mcpTool('session:manager-current', 'list_dispatches', {});
    expect(
      visible.value.dispatches.find(
        (r: { dispatchId: string }) => r.dispatchId === record.dispatchId,
      ).note,
    ).toBe(record.note);
    const unknown = structuredClone(record);
    const journal = path.join(configDir, 'remote-dispatches.json');
    const inode = fs.statSync(journal).ino;
    await reconnect('replay-unknown');
    expect(record).toEqual(unknown);
    expect(fs.statSync(journal).ino).toBe(inode);
    expect(delivery.mock.calls).toHaveLength(beforeWakes);
    expect(launch.mock.calls).toHaveLength(localLaunches);
    expect(await (await fetch(ready.control + '/evidence')).json()).toHaveLength(1);
    expect(
      history.task(task.taskId)!.workflow!.steps.find((s) => s.id === 'implement')?.state,
    ).toBe('dispatched');
    await reconnect('replay-known');
    expect(record).toEqual(unknown);

    await fetch(ready.control + '/control', {
      method: 'POST',
      body: JSON.stringify({ kind: 'progress' }),
    });
    await vi.waitFor(
      () =>
        expect(
          delivery.mock.calls.some(
            ([id, text]) => id === 'manager-current' && text.includes('remote fixture progress'),
          ),
        ).toBe(true),
      { timeout: 10_000 },
    );
    await fetch(ready.control + '/control', {
      method: 'POST',
      body: JSON.stringify({ kind: 'block' }),
    });
    await vi.waitFor(
      () =>
        expect(
          delivery.mock.calls.some(
            ([id, text]) => id === 'manager-current' && text.includes('blocked on a decision'),
          ),
        ).toBe(true),
      { timeout: 10_000 },
    );
    expect(
      history.task(task.taskId)!.workflow!.steps.find((s) => s.id === 'implement')?.state,
    ).toBe('blocked');
    sessions.reparentChildren('manager-current', 'manager-other');
    pairedWorkerConnection.stop();
    const reply = 'Remote task complete.\n```wks-result\n{"commit":"fixture-commit"}\n```';
    await fetch(ready.control + '/control', {
      method: 'POST',
      body: JSON.stringify({ kind: 'finish', reply }),
    });
    await new Promise((resolve) => setTimeout(resolve, 100));
    await pairedWorkerConnection.connect();
    await vi.waitFor(
      () =>
        expect(
          persisted()
            .flatMap((t) => t.attempts)
            .find((a) => a.sessionId === result.value.sessionId)?.resultContract,
        ).toBe('valid'),
      { timeout: 10_000 },
    );
    const wakes = () =>
      delivery.mock.calls.filter(
        ([id, text]) => id === 'manager-other' && text.includes('fixture-commit'),
      );
    await vi.waitFor(() => expect(wakes()).toHaveLength(1));
    expect(wakes()[0][1]).toContain('not a new user request');
    const retained = persisted().find((t) => t.taskId === task.taskId)!;
    expect(retained.sources?.[0].requestId).toBe(request.requestId);
    expect(retained.ownerSessionId).toBe('manager-other');
    expect(retained.attempts[0].worktree?.directoryIdentity).toBeUndefined();
    expect(retained.workflow?.hash).toBe(task.workflow?.hash);
    expect(retained.workflow?.steps.find((s) => s.id === 'implement')?.state).toBe('completed');
    await pairedWorkerConnection.call('agents.dispatchReplay', { dispatchId: record.dispatchId });
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(wakes()).toHaveLength(1);
    const completed = structuredClone(record);
    await reconnect('replay-unknown');
    expect(record).toEqual(completed);
    expect(record.state).toBe('done');
    expect(wakes()).toHaveLength(1);
    expect(await (await fetch(ready.control + '/evidence')).json()).toHaveLength(1);

    // A separate real dispatch retains its terminal packet, then its manager
    // send fails. Do not manufacture this state by reopening an acknowledged
    // receipt: exercise retainEvidence -> beginDelivery -> uncertain send.
    await fetch(ready.control + '/control', {
      method: 'POST',
      body: JSON.stringify({ kind: 'replay-known' }),
    });
    const nextRequest = inbox.prepare('manager-other', 'Return the retained outcome fixture.');
    if (!nextRequest.available) throw new Error('Request capture unavailable');
    const nextSend = inbox.beginDelivery('manager-other', nextRequest.requestId)!;
    inbox.finishDelivery(nextRequest.requestId, nextSend.deliveryId, 'accepted');
    const next = await mcpTool('session:manager-other', 'resolve_manager_request', {
      requestId: nextRequest.requestId,
      expectedRevision: inbox.request('manager-other', nextRequest.requestId).revision,
      intents: [
        {
          key: 'retained-outcome-task',
          kind: 'create',
          cwd: project,
          title: 'Retained outcome with uncertain manager wake',
          provenance: 'explicit',
          reason: 'User requested the retained outcome fixture',
        },
      ],
    });
    expect(next.isError, next.text).toBe(false);
    expect(next.value.ok, next.text).toBe(true);
    await mcpTool('session:manager-other', 'decide_workflow_step', {
      cwd: project,
      taskId: next.value.tasks[0].taskId,
      stepId: 'scout',
      run: false,
      reason: 'Exercise the existing paired result receipt',
    });
    const uncertain = await mcpSpawn('session:manager-other', {
      provider: 'claude',
      model: route.value.model,
      capability: route.value.capability,
      decisionId: route.value.decisionId,
      role: 'implementer',
      executionTarget: 'paired',
      remoteCwd: ready.repo,
      parentSessionId: 'manager-other',
      taskId: next.value.tasks[0].taskId,
      workflowStepId: 'implement',
      stage: 'implement',
      template: 'ship-task',
      templateParams: { task: 'Return the retained outcome fixture' },
      toolScope: 'view',
    });
    expect(uncertain.isError, uncertain.text).toBe(false);
    const pending = remoteDispatchRegistry
      .list()
      .find((r) => r.localSessionId === uncertain.value.sessionId)!;
    delivery.mockRejectedValueOnce(new Error('fixture: manager wake delivery uncertain'));
    await fetch(ready.control + '/control', {
      method: 'POST',
      body: JSON.stringify({
        kind: 'finish',
        reply: 'Known terminal outcome.\n```wks-result\n{"commit":"retained-outcome"}\n```',
      }),
    });
    await vi.waitFor(
      () => {
        expect(pending.lastUpdate?.final).toBe(true);
        expect(pending.deliveringSeq).toEqual(expect.any(Number));
        expect(pending.deliveringSeq).toBe(pending.lastUpdate?.seq);
        expect(pending.note).toContain('delivery is unknown');
      },
      { timeout: 10_000 },
    );
    const pendingBefore = structuredClone(pending);
    const terminalEvidence = pending.lastUpdate;
    const attempts = delivery.mock.calls.length;
    expect(pendingBefore.state).toBe('open');
    expect(pendingBefore.ackedSeq).toBe(0);
    const journalBefore = fs.readFileSync(journal, 'utf8');
    fs.rmSync(journal);
    fs.mkdirSync(journal);
    await reconnect('replay-unknown', pending);
    expect(pending).toEqual(pendingBefore);
    expect(pending.lastUpdate).toBe(terminalEvidence);
    fs.rmdirSync(journal);
    fs.writeFileSync(journal, journalBefore);
    await reconnect('replay-unknown', pending);
    expect(pending.note).toContain('terminal outcome is retained locally');
    expect(pending.note).toContain('wake delivery is unconfirmed');
    expect(pending.note).not.toContain('Worker outcome is unknown');
    expect(pending).toEqual({ ...pendingBefore, note: pending.note });
    expect(pending.lastUpdate).toBe(terminalEvidence);
    const precise = structuredClone(pending);
    const preciseInode = fs.statSync(journal).ino;
    await reconnect('replay-unknown', pending);
    expect(pending).toEqual(precise);
    expect(fs.statSync(journal).ino).toBe(preciseInode);
    const saved = JSON.parse(fs.readFileSync(journal, 'utf8')).find(
      (r: { dispatchId: string }) => r.dispatchId === pending.dispatchId,
    );
    expect(saved.lastUpdate).toEqual(terminalEvidence);
    expect(saved.deliveringSeq).toBe(pendingBefore.deliveringSeq);
    expect(saved.note).toBe(precise.note);
    const accepts = vi.spyOn(remoteDispatchRegistry, 'accept');
    try {
      await reconnect('replay-known', pending);
      await vi.waitFor(() =>
        expect(
          accepts.mock.results.some(
            (r, i) =>
              (accepts.mock.calls[i][1] as { dispatchId?: string })?.dispatchId ===
                pending.dispatchId &&
              r.type === 'return' &&
              !r.value.ok &&
              r.value.reason === 'delivery-unknown',
          ),
        ).toBe(true),
      );
    } finally {
      accepts.mockRestore();
    }
    expect(pending).toEqual(precise);
    expect(pending.lastUpdate).toBe(terminalEvidence);
    expect(delivery.mock.calls).toHaveLength(attempts);
    expect(launch.mock.calls).toHaveLength(localLaunches);
    expect(await (await fetch(ready.control + '/evidence')).json()).toHaveLength(2);
    expect(record).toEqual(completed);
    const sourceBefore = execFileSync('git', ['-C', project, 'status', '--porcelain=v1']);
    const transferRequest = inbox.prepare(
      'manager-other',
      'Transfer the exact checkpoint and required report, then review locally.',
    );
    if (!transferRequest.available) throw new Error('Request capture unavailable');
    const transferSend = inbox.beginDelivery('manager-other', transferRequest.requestId)!;
    inbox.finishDelivery(transferRequest.requestId, transferSend.deliveryId, 'accepted');
    const transferTask = await mcpTool('session:manager-other', 'resolve_manager_request', {
      requestId: transferRequest.requestId,
      expectedRevision: inbox.request('manager-other', transferRequest.requestId).revision,
      intents: [
        {
          key: 'exact-handoff',
          kind: 'create',
          cwd: project,
          title: 'Exact handoff and local review',
          provenance: 'explicit',
          reason: 'User requested exact code and report custody',
        },
      ],
    });
    expect(transferTask.isError, transferTask.text).toBe(false);
    const transferTaskId = transferTask.value.tasks[0].taskId;
    await mcpTool('session:manager-other', 'decide_workflow_step', {
      cwd: project,
      taskId: transferTaskId,
      stepId: 'scout',
      run: false,
      reason: 'Fixture supplies verified scout evidence',
    });
    const exact = await mcpSpawn('session:manager-other', {
      provider: 'claude',
      model: route.value.model,
      capability: route.value.capability,
      decisionId: route.value.decisionId,
      role: 'implementer',
      executionTarget: 'paired',
      remoteCwd: ready.repo,
      parentSessionId: 'manager-other',
      taskId: transferTaskId,
      workflowStepId: 'implement',
      stage: 'implement',
      worktree: true,
      template: 'ship-task',
      templateParams: {
        task: 'Implement using the verified scout evidence. Return the required report.',
      },
      toolScope: 'operator',
      taskSource: {
        binding: ready.handoffBinding,
        artifacts: [{ name: 'scout.md', kind: 'report' }],
        outputs: [{ name: 'implementation.md', kind: 'report' }],
      },
    });
    expect(exact.isError, exact.text).toBe(false);
    const exactRecord = remoteDispatchRegistry
      .list()
      .find((r) => r.localSessionId === exact.value.sessionId)!;
    expect(exactRecord.handoff?.state).toBe('prepared');
    const actualLaunches = (await (await fetch(ready.control + '/evidence')).json()) as Array<{
      cwd: string;
    }>;
    const execution = actualLaunches.at(-1)!;
    expect(
      execFileSync('git', ['-C', execution.cwd, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
    ).toBe(ready.sourceCommit);
    expect(
      fs.readFileSync(
        path.join(execution.cwd, '.workspacer', 'handoffs', exactRecord.dispatchId, 'scout.md'),
        'utf8',
      ),
    ).toContain('Task evidence');
    await fetch(ready.control + '/control', {
      method: 'POST',
      body: JSON.stringify({
        kind: 'handoff-result',
        reply:
          'Reported test claim: passed.\n```wks-result\n{"commit":"worker-claimed-commit"}\n```',
      }),
    });
    await vi.waitFor(() => expect(exactRecord.state).toBe('done'), { timeout: 20000 });
    expect(exactRecord.handoff?.state).toBe('received');
    const imported = history
      .task(exact.value.taskId)!
      .attempts.find((a) => a.sessionId === exact.value.sessionId)!;
    expect(imported.handoff?.head).not.toBe(ready.sourceCommit);
    expect(imported.handoff?.base).toBe(ready.sourceCommit);
    expect(imported.executionHost).toBe('Paired fixture workspace');
    expect(imported.reviewEvidenceId).toEqual(expect.any(String));
    expect(
      history.openTarget({
        taskId: exact.value.taskId,
        kind: 'handoff',
        dispatchId: imported.dispatchId,
        artifact: 0,
      }).target,
    ).toContain('implementation.md');
    expect(execFileSync('git', ['-C', project, 'status', '--porcelain=v1'])).toEqual(sourceBefore);
    expect(
      execFileSync('git', ['-C', project, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
    ).toBe(ready.sourceCommit);
    const localRoute = await mcpTool('session:manager-other', 'select_model', {
      role: 'reviewer',
      provider: 'codex',
      cwd: project,
    });
    expect(localRoute.isError, localRoute.text).toBe(false);
    const continuation = {
      provider: localRoute.value.provider,
      model: localRoute.value.model,
      capability: localRoute.value.capability,
      decisionId: localRoute.value.decisionId,
      role: 'reviewer',
      parentSessionId: 'manager-other',
      taskId: exact.value.taskId,
      afterDispatchId: imported.dispatchId,
      workflowStepId: 'review',
      stage: 'review',
      template: 'review-task',
      templateParams: {
        task: 'Review the returned checkpoint locally.',
        handoff: 'Use the host-transferred implementation report.',
      },
      toolScope: 'view',
      taskSource: {
        binding: ready.handoffBinding,
        artifacts: [{ name: 'implementation.md', kind: 'report' }],
        outputs: [],
      },
    };
    const continued = await mcpSpawn('session:manager-other', continuation);
    expect(continued.isError, continued.text).toBe(false);
    const localExecution = launch.mock.lastCall![0];
    expect(
      execFileSync('git', ['-C', localExecution.cwd, 'rev-parse', 'HEAD'], {
        encoding: 'utf8',
      }).trim(),
    ).toBe(imported.handoff!.head);
    const evidenceFolder = fs.readdirSync(path.join(localExecution.cwd, '.workspacer/handoffs'))[0];
    expect(
      fs.readFileSync(
        path.join(localExecution.cwd, '.workspacer/handoffs', evidenceFolder, 'implementation.md'),
        'utf8',
      ),
    ).toContain('Result evidence');
    const launchCount = launch.mock.calls.length;
    expect((await mcpSpawn('session:manager-other', continuation)).isError).toBe(true);
    expect(launch.mock.calls).toHaveLength(launchCount);
  } finally {
    setRemoteServer(null);
    pairedWorkerConnection.stop();
    pairedCalls.mockRestore();
    delivery.mockRestore();
    const exited = once(peer, 'exit');
    peer.stdin.end();
    await exited;
  }
}, 180_000);
