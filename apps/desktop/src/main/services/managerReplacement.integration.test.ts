/** Real desktop service, manager spawn, tokens, session/task stores and wake
 * coalescer. Only external provider/daemon/OS integrations are fixtures. */
import { afterAll, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { artifactHash } from './managerReplacementArtifact';
import { WORKFLOW_STARTERS } from '../shared/fleetWorkflow';

const rig = vi.hoisted(() => ({
  root: '',
  paneSession: '',
  fault: '',
  messages: [] as Array<[string, string]>,
  spawns: [] as any[],
  wire: new Map<string, any>(),
}));
vi.mock('electron', () => ({ BrowserWindow: class {}, app: { getPath: () => rig.root } }));
vi.mock('./hubClient', () => ({ ownsHubCapability: () => true }));
vi.mock('./agentRuntimeStatus', () => ({
  readAgentRuntimeStatus: async () => ({ claudemon: 'ready', hub: 'ready', facade: 'ready' }),
}));
vi.mock('./hubDaemon', () => ({ isHubAdopted: () => false }));
vi.mock('./remoteServer', () => ({
  getRemoteServer: () => null,
  isRemoteClientMode: () => false,
}));
vi.mock('./directCompletion', () => ({
  completeReadinessPing: vi.fn(async () => ({ ok: true, text: 'OK' })),
}));
vi.mock('./configService', () => ({
  getConfigDir: () => rig.root,
  configService: {
    getConfig: () => ({
      agents: {},
      claude: { transport: 'stream' },
      codex: { transport: 'stream' },
    }),
    saveConfig: vi.fn(),
    onChange: vi.fn(),
  },
}));
vi.mock('./agentProviders', () => ({
  resolveAgentBinary: () => '/fixture/provider',
  isAgentBinaryInstalled: () => true,
  checkAllProviders: () => [{ provider: 'claude', resolvedPath: '/fixture/provider' }],
}));
vi.mock('./claudeProfiles', () => ({
  claudeProfiles: { getProfile: () => undefined, getProfiles: () => [] },
  scrubBypassProfile: (p: unknown) => p,
  scrubRemoteGrantedProfile: (p: unknown) => p,
}));
vi.mock('./claudeAccountSetup', () => ({ syncAccountTrust: vi.fn() }));
vi.mock('./claudeEffortDefault', () => ({ resolveClaudeDefaultEffort: () => undefined }));
vi.mock('./launchIntegrations', () => ({
  prepareLaunchIntegration: async (_id: unknown, _ctx: unknown, prepared: unknown) => prepared,
}));
vi.mock('./libraryService', () => ({ libraryService: { list: () => [] } }));
vi.mock('./managerSkills', () => ({ installManagerSkills: vi.fn() }));
vi.mock('./responseCardSkill', () => ({ installResponseCardSkill: () => '' }));
vi.mock('./systemNotice', () => ({ notifySystem: vi.fn() }));
vi.mock('./mcpConfig', () => ({
  MCP_FACADE_URL: 'http://127.0.0.1:0',
  managedFacadeInstructions: () => 'Manager facade instructions',
  buildSessionMcpConfig: () => null,
  facadeSessionMcpConfig: () => path.join(rig.root, 'mcp.json'),
  facadeUrlWithToken: () => 'http://127.0.0.1:0',
}));
vi.mock('./claudemonDaemon', () => ({
  CLAUDEMON_API_URL: 'http://127.0.0.1:0',
  claudemonOverlayPath: () => '',
  claudeSettingsOverlayEnabled: () => false,
  getClaudemonReadinessOwner: () => 'fixture-owned-daemon',
}));
vi.mock('./agentNotifier', () => ({ agentNotifier: { notifyOnTransition: vi.fn() } }));
vi.mock('./workflowWatcher', () => ({
  workflowWatcher: { attach: vi.fn(), detach: vi.fn(), poke: vi.fn() },
}));
vi.mock('./hubTelemetry', () => ({
  publishWorkflowRuns: vi.fn(),
  publishSnapshot: vi.fn(),
  forgetSession: vi.fn(),
}));
vi.mock('./sessionStore/analyticsWriter', () => ({ writeHistory: vi.fn() }));
vi.mock('./fleetReviewStore', () => ({
  fleetReviewStore: { capture: async () => undefined, adoptOwner: vi.fn() },
}));
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: {
    attachedSession: () => rig.paneSession,
    spawnManaged: async (request: any) => {
      rig.spawns.push(request);
      rig.wire.set(request.sessionId, {
        session_id: request.sessionId,
        cwd: request.cwd,
        provider: request.provider,
        mode: 'input',
        user_prompts: 0,
      });
      const wire = rig.wire.get(request.sessionId);
      if (rig.fault === 'identity') wire.session_id = 'wrong-identity';
      if (rig.fault === 'cwd') wire.cwd = path.join(rig.root, 'wrong-directory');
      if (rig.fault === 'provider') wire.provider = 'claude';
      if (rig.fault === 'grants') {
        const filename = path.join(rig.root, 'tokens.json');
        const tokens = JSON.parse(fs.readFileSync(filename, 'utf8'));
        tokens.find((t: any) => t.label === `session:${request.sessionId}`).yoloAllowed = true;
        fs.writeFileSync(filename, JSON.stringify(tokens));
      }
      return request.sessionId;
    },
    getSession: async (id: string) => rig.wire.get(id) ?? null,
    getConversation: async (id: string) => ({
      seq: sequences.get(id) ?? 0,
      items: (claudeSessionStore.getSnapshot(id)?.conversation ?? []).map((t) => ({
        type: t.role === 'user' ? 'user_message' : 'assistant_text',
        text: t.content,
      })),
    }),
    signal: async (id: string, signal: string) => {
      if (signal === 'SIGTERM' && rig.wire.has(id)) rig.wire.get(id).mode = 'stopped';
    },
    messageDirect: async (id: string, text: string) => send(id, text),
    message: async (id: string, text: string) =>
      managerReplacementState.holdMessage(id, text) ? { ok: true } : send(id, text),
  },
}));

import { managerReplacementService } from './managerReplacement';
import { managerReplacementState, ManagerReplacementState } from './managerReplacementState';
import { spawnManagedAgent } from './managedSpawn';
import { claudeSessionStore } from './claudeSessionStore';
import { dispatchHistoryStore } from './dispatchHistoryStore';
import { supervisorNudge } from './supervisorNudge';
import { ownerTask } from './fleetWorkflowRuntime';
import { sessionFacadeGrantFingerprint } from './remoteTokens';
import { installManagerSkills } from './managerSkills';
import { startProviderReadiness, providerReadinessService } from './providerReadinessRuntime';
import { completeReadinessPing } from './directCompletion';

const sequences = new Map<string, number>();
function say(id: string, role: 'user' | 'assistant', text: string) {
  const sequence = (sequences.get(id) ?? 0) + 1;
  sequences.set(id, sequence);
  claudeSessionStore.applyConversationDelta({
    session_id: id,
    seq: sequence,
    reset: false,
    items: [{ type: role === 'user' ? 'user_message' : 'assistant_text', text }] as never,
  });
}
async function send(id: string, text: string) {
  rig.messages.push([id, text]);
  if (text.startsWith('HOST-OWNED')) {
    const op = managerReplacementState.related(id)!;
    const brief = path.join(rig.root, '.workspacer', 'brief.md');
    fs.writeFileSync(brief, 'Checkpointed pending approval and task priorities.');
    const raw = JSON.stringify({
      version: 1,
      operationId: op.operationId,
      sourceSessionId: id,
      cwd: rig.root,
      checkpoint: {
        completed: true,
        files: [{ path: brief, sha256: artifactHash(fs.readFileSync(brief, 'utf8')) }],
      },
      workers: op.workerIds.map((sessionId) => ({
        sessionId,
        instructions: 'Keep the requested task and pending approvals',
      })),
      tasks: op.taskIds.map((taskId) => ({
        taskId,
        nextAction: 'Inspect current pinned steps and waivers before continuing',
      })),
      pendingDecisions: ['Deployment requires a decision'],
      facts: ['Do not change the pinned review policy'],
      nextAction: 'Receive the workers',
    });
    fs.writeFileSync(op.artifactPath, raw);
    say(id, 'user', text);
    say(
      id,
      'assistant',
      '```wks-manager-handoff\n' +
        JSON.stringify({
          operationId: op.operationId,
          sourceSessionId: id,
          sha256: artifactHash(raw),
        }) +
        '\n```',
    );
  }
  claudeSessionStore.applyManagedMode(id, 'input', { provider: 'codex', transport: 'stream' });
  return { ok: true };
}
afterAll(() => {
  providerReadinessService.dispose();
  if (rig.root) fs.rmSync(rig.root, { recursive: true, force: true });
});

it('runs the real local transaction without resuming, transferring both task kinds, preserving grants and routing coalesced/next-turn finishes', async () => {
  rig.root = fs.mkdtempSync(path.join(os.tmpdir(), 'replacement-integrated-'));
  const source = await spawnManagedAgent({
    provider: 'codex',
    cwd: rig.root,
    model: 'gpt-5.5',
    effort: 'high',
    contextWindow: 200000,
    toolScope: 'operator',
    manager: true,
    transport: 'stream',
    label: 'Fleet Manager',
  });
  rig.paneSession = source;
  const worker = await spawnManagedAgent({
    provider: 'codex',
    cwd: rig.root,
    parentSessionId: source,
    firstMessage: 'Implement assigned task',
  });
  const pending = 'pending-fixture-worker';
  claudeSessionStore.setSpawnMeta(pending, { parentSessionId: source, label: 'Still registering' });
  const owner = claudeSessionStore.getSnapshot(source)!;
  const ordinary = dispatchHistoryStore.accept({
    owner,
    projectCwd: rig.root,
    executionCwd: rig.root,
    sessionId: worker,
    title: 'Ordinary dispatch',
  })!;
  const definition = structuredClone(WORKFLOW_STARTERS[0]);
  const task = dispatchHistoryStore.startWorkflow(owner, rig.root, 'Workflow dispatch', {
    definition,
    hash: 'pinned',
    templates: {},
    steps: definition.steps.map((s) => ({ id: s.id, state: 'planned' })),
  });
  // Hold real cross-process admission while Inspector edits a future step.
  dispatchHistoryStore.workflowDecision(task.taskId, 'scout', false, 'Scope verified');
  const reservation = dispatchHistoryStore.reserveWorkflowDispatch(
    task.taskId,
    dispatchHistoryStore.task(task.taskId)!.revision!,
    'implement',
  );
  expect(
    dispatchHistoryStore.editByHostUser(
      {
        taskId: task.taskId,
        expectedTaskRevision: dispatchHistoryStore.task(task.taskId)!.revision!,
        action: 'waive',
        stepId: 'review',
        reason: 'User-approved review exception',
      },
      () => undefined,
      () => true,
    ).ok,
  ).toBe(true);
  expect(
    dispatchHistoryStore.editByHostUser(
      {
        taskId: task.taskId,
        expectedTaskRevision: dispatchHistoryStore.task(task.taskId)!.revision!,
        action: 'links',
        links: { pullRequest: { number: '42', url: 'https://example.test/pull/42' } },
      },
      () => undefined,
      () => true,
    ).ok,
  ).toBe(true);
  const manualCandidate = await spawnManagedAgent({
    provider: 'codex',
    cwd: rig.root,
    manager: true,
    toolScope: 'operator',
    transport: 'stream',
  });
  const historyPath = path.join(rig.root, 'dispatch-history.json');
  const reservedTasks = fs.readFileSync(historyPath, 'utf8');
  expect(() => claudeSessionStore.reparentChildren(source, manualCandidate)).toThrow('reservation');
  expect(fs.readFileSync(historyPath, 'utf8')).toBe(reservedTasks);
  const workflowWorker = await spawnManagedAgent({
    provider: 'codex',
    cwd: rig.root,
    parentSessionId: source,
  });
  const spawnsBeforeWait = rig.spawns.length;
  const response = await managerReplacementService.request({
    action: 'start',
    sourceSessionId: source,
    paneId: 'same-pane',
    workspaceId: 'same-workspace',
  });
  const waitingOperation = response.operations.at(-1)!;
  await new Promise((r) => setTimeout(r, 30));
  expect(managerReplacementState.get(waitingOperation.operationId)).toMatchObject({
    phase: 'preparing',
    committed: false,
  });
  expect(rig.spawns).toHaveLength(spawnsBeforeWait);
  for (const task of dispatchHistoryStore.list()) expect(task.ownerSessionId).toBe(source);
  expect(claudeSessionStore.getSnapshot(worker)?.parentSessionId).toBe(source);
  expect(managerReplacementState.metadata(pending)?.parentSessionId).toBe(source);
  dispatchHistoryStore.accept({
    owner,
    projectCwd: rig.root,
    executionCwd: rig.root,
    sessionId: workflowWorker,
    taskId: task.taskId,
    workflowStepId: 'implement',
    stage: 'implement',
  });
  dispatchHistoryStore.releaseWorkflowDispatch(task.taskId, reservation);
  const before = dispatchHistoryStore.task(task.taskId)!;
  const grants = sessionFacadeGrantFingerprint(source);
  // A finish already inside the coalesce window when the user clicks.
  say(worker, 'user', 'Implement assigned task');
  say(worker, 'assistant', 'First result');
  claudeSessionStore.applyManagedMode(worker, 'input', { provider: 'codex', transport: 'stream' });
  supervisorNudge.onFinished(claudeSessionStore.getSnapshot(worker)!, source, 'First result');
  expect(response.error).toBeUndefined();
  const id = response.operations.at(-1)!.operationId;
  await managerReplacementService.idle(id);
  const op = managerReplacementState.get(id);
  expect(op).toMatchObject({ phase: 'binding', committed: true });
  const spawn = rig.spawns.at(-1);
  expect(spawn).toMatchObject({
    sessionId: op.successorSessionId,
    provider: 'codex',
    model: 'gpt-5.5',
    effort: 'high',
    contextWindow: 200000,
  });
  expect(spawn.resumeSessionId).toBeUndefined();
  expect(spawn.firstMessage).toBeUndefined();
  expect(installManagerSkills).toHaveBeenCalledWith('codex', true);
  expect(sessionFacadeGrantFingerprint(op.successorSessionId)).toBe(grants);
  expect(claudeSessionStore.getSnapshot(worker)?.parentSessionId).toBe(op.successorSessionId);
  expect(dispatchHistoryStore.task(ordinary.taskId)?.ownerSessionId).toBe(op.successorSessionId);
  expect(dispatchHistoryStore.task(task.taskId)).toMatchObject({
    ownerSessionId: op.successorSessionId,
    workflow: before.workflow,
    attempts: before.attempts.map(({ dispatchId, sessionId, acceptedAt, workflowStepId }) => ({
      dispatchId,
      sessionId,
      acceptedAt,
      workflowStepId,
    })),
    links: before.links,
    audit: before.audit,
    revision: expect.any(Number),
  });
  expect(dispatchHistoryStore.task(task.taskId)!.revision).toBeGreaterThan(before.revision!);
  expect(() => ownerTask(task.taskId, source, rig.root)).toThrow();
  expect(() => ownerTask(task.taskId, op.successorSessionId, rig.root)).toThrow('fenced');
  // Run the actual desktop startup scheduler while successor ownership is
  // committed but pane activation is pending. Its isolated inference adapter
  // must not submit any manager message or bypass the successor fence.
  startProviderReadiness();
  await new Promise((r) => setTimeout(r, 2100));
  expect(completeReadinessPing).toHaveBeenCalledTimes(1);
  expect(completeReadinessPing).toHaveBeenCalledWith(
    'claude',
    '/fixture/provider',
    expect.any(AbortSignal),
  );
  expect(rig.messages.filter(([target]) => target === op.successorSessionId)).toHaveLength(0);
  expect(
    managerReplacementState.get(id).deliveries.find((d) => d.kind === 'message'),
  ).toMatchObject({ status: 'pending' });
  expect(
    rig.messages.filter(
      ([id, text]) => id === op.successorSessionId && text.includes('First result'),
    ),
  ).toHaveLength(0);
  rig.paneSession = op.successorSessionId;
  await managerReplacementService.request({ action: 'bind', operationId: id });
  await managerReplacementService.idle(id);
  expect(managerReplacementState.get(id).phase).toBe('complete');
  expect(ownerTask(task.taskId, op.successorSessionId, rig.root).taskId).toBe(task.taskId);
  expect(rig.wire.get(source).mode).toBe('stopped');
  expect(
    rig.messages.some(
      ([id, text]) =>
        id === op.successorSessionId && text.includes('Deployment requires a decision'),
    ),
  ).toBe(true);
  expect(
    rig.messages.filter(
      ([id, text]) => id === op.successorSessionId && text.includes('First result'),
    ),
  ).toHaveLength(1);
  // A repeat edge is suppressed; an actual later result still gets through.
  supervisorNudge.onFinished(
    claudeSessionStore.getSnapshot(worker)!,
    op.successorSessionId,
    'First result',
  );
  say(worker, 'assistant', 'Second result');
  claudeSessionStore.applyManagedMode(worker, 'input', { provider: 'codex', transport: 'stream' });
  supervisorNudge.onFinished(
    claudeSessionStore.getSnapshot(worker)!,
    op.successorSessionId,
    'Second result',
  );
  claudeSessionStore.ensureManagedSession(pending, rig.root);
  expect(claudeSessionStore.getSnapshot(pending)?.parentSessionId).toBe(op.successorSessionId);
  await new Promise((r) => setTimeout(r, 1700));
  expect(
    rig.messages.filter(
      ([id, text]) => id === op.successorSessionId && text.includes('Second result'),
    ),
  ).toHaveLength(1);
  for (const fault of ['identity', 'cwd', 'provider', 'grants']) {
    rig.fault = fault;
    const refused = await managerReplacementService.request({
      action: 'start',
      sourceSessionId: op.successorSessionId,
      paneId: 'same-pane',
      workspaceId: 'same-workspace',
    });
    const attempt = refused.operations.at(-1)!;
    await managerReplacementService.idle(attempt.operationId);
    expect(managerReplacementState.get(attempt.operationId)).toMatchObject({
      phase: 'failed',
      committed: false,
    });
    expect(dispatchHistoryStore.task(ordinary.taskId)?.ownerSessionId).toBe(op.successorSessionId);
    expect(claudeSessionStore.getSnapshot(worker)?.parentSessionId).toBe(op.successorSessionId);
    expect(rig.wire.get(op.successorSessionId).mode).toBe('input');
  }
  rig.fault = '';
  const laterWorker = await spawnManagedAgent({
    provider: 'codex',
    cwd: rig.root,
    parentSessionId: op.successorSessionId,
    firstMessage: 'A later task',
  });
  expect(
    new ManagerReplacementState(() => path.join(rig.root, 'manager-replacements.json')).metadata(
      laterWorker,
    )?.parentSessionId,
  ).toBe(op.successorSessionId);
  // The current successor can use normal settings/restart controls; only the
  // retired predecessor and incomplete operations are fenced from resuming.
  expect(() => managerReplacementState.assertResume(source)).toThrow();
  expect(() => managerReplacementState.assertResume(op.successorSessionId)).not.toThrow();
  claudeSessionStore.noteEffort(op.successorSessionId, 'medium');
  expect(managerReplacementState.launch(op.successorSessionId)?.options.effort).toBe('medium');
  const next = await managerReplacementService.request({
    action: 'start',
    sourceSessionId: op.successorSessionId,
    paneId: 'same-pane',
    workspaceId: 'same-workspace',
  });
  const nextId = next.operations.at(-1)!.operationId;
  await managerReplacementService.idle(nextId);
  const nextOp = managerReplacementState.get(nextId);
  expect(nextOp.phase).toBe('binding');
  expect(rig.spawns.at(-1).effort).toBe('medium');
  rig.paneSession = nextOp.successorSessionId;
  await managerReplacementService.request({ action: 'bind', operationId: nextId });
  await managerReplacementService.idle(nextId);
  expect(managerReplacementState.get(nextId).phase).toBe('complete');
  const manual = await spawnManagedAgent({
    provider: 'codex',
    cwd: rig.root,
    model: 'gpt-5.5',
    effort: 'medium',
    manager: true,
    toolScope: 'operator',
    transport: 'stream',
  });
  claudeSessionStore.reparentChildren(nextOp.successorSessionId, manual);
  const recoveredMetadata = new ManagerReplacementState(() =>
    path.join(rig.root, 'manager-replacements.json'),
  );
  expect(recoveredMetadata.metadata(worker)?.parentSessionId).toBe(manual);
  expect(recoveredMetadata.metadata(manual)?.isWakeTarget).toBe(true);
  expect(recoveredMetadata.wakeTarget(source)).toBe(manual);
  expect(dispatchHistoryStore.task(ordinary.taskId)?.ownerSessionId).toBe(manual);
}, 10000);
