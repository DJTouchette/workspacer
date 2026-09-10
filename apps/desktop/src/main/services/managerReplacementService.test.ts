import { afterEach, describe, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  ManagerReplacementState,
  type ManagerLaunch,
  type ReplacementMetadata,
} from './managerReplacementState';
import { ManagerReplacementService, type ReplacementHost } from './managerReplacementService';
import { artifactHash, validateManagerArtifact } from './managerReplacementArtifact';
import { ManagerDeliveryRejected } from '../shared/managerReplacement';

const dirs: string[] = [];
afterEach(() => {
  dirs.splice(0).forEach((d) => fs.rmSync(d, { recursive: true, force: true }));
  vi.restoreAllMocks();
});
function fixture() {
  const cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'replacement-test-'));
  dirs.push(cwd);
  fs.mkdirSync(path.join(cwd, '.workspacer'));
  const brief = path.join(cwd, '.workspacer', 'brief.md');
  fs.writeFileSync(brief, 'Checkpointed state');
  const filename = path.join(cwd, 'journal.json');
  const state = new ManagerReplacementState(() => filename);
  const launch: ManagerLaunch = {
    options: {
      provider: 'codex',
      cwd,
      model: 'model',
      effort: 'high',
      contextWindow: 200000,
      permissionMode: 'ask',
      manager: true,
      toolScope: 'operator',
      transport: 'stream',
    },
    grants: 'grant',
  };
  let metadata: ReplacementMetadata[] = [
    { sessionId: 'old', cwd, isWakeTarget: true, provider: 'codex' },
    {
      sessionId: 'worker',
      cwd,
      parentSessionId: 'old',
      label: 'Worker',
      routing: { role: 'implementer' },
    },
    { sessionId: 'pending', cwd, parentSessionId: 'old' },
  ];
  let owner = 'old';
  let receipt = '';
  let bound = false;
  const sent: Array<{ id: string; text: string }> = [];
  const host: ReplacementHost = {
    source: vi.fn(() => structuredClone(launch)),
    inventory: vi.fn(() => structuredClone(metadata)),
    tasks: vi.fn((id) => (id === owner ? ['ordinary-task', 'workflow-task'] : [])),
    readyForTransfer: () => true,
    signatures: () => ({ previouslyReported: 'unchanged' }),
    finishes: () => ({}),
    receipt: () => receipt,
    settled: () => true,
    spawn: vi.fn(async () => {}),
    validateSuccessor: vi.fn(async () => {}),
    transfer: vi.fn((old, next) => {
      owner = next;
      metadata = metadata.map((m) =>
        m.parentSessionId === old ? { ...m, parentSessionId: next } : m,
      );
    }),
    restore: vi.fn(async () => {}),
    bound: () => bound,
    send: vi.fn(async (id, text) => {
      sent.push({ id, text });
      if (text.startsWith('HOST-OWNED')) {
        const op = state.records()[0];
        const artifact = {
          version: 1,
          operationId: op.operationId,
          sourceSessionId: 'old',
          cwd,
          checkpoint: {
            completed: true,
            files: [{ path: brief, sha256: artifactHash(fs.readFileSync(brief, 'utf8')) }],
          },
          workers: op.workerIds.map((sessionId) => ({
            sessionId,
            instructions: 'Exact assigned task and pending constraints',
          })),
          tasks: op.taskIds.map((taskId) => ({ taskId, nextAction: 'Continue after review' })),
          pendingDecisions: ['Deployment approval is pending'],
          facts: ['User asked for tests'],
          nextAction: 'Wait for worker result',
        };
        const raw = JSON.stringify(artifact);
        fs.writeFileSync(op.artifactPath, raw);
        receipt =
          '```wks-manager-handoff\n' +
          JSON.stringify({
            operationId: op.operationId,
            sourceSessionId: 'old',
            sha256: artifactHash(raw),
          }) +
          '\n```';
      }
      return { ok: true };
    }),
    pause: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    kickoff: () => 'Successor kickoff',
    recoverFinishes: vi.fn(),
  };
  const service = new ManagerReplacementService(state, host, {
    preparationMs: 50,
    pollMs: 1,
    deliveryMs: 50,
  });
  const start = async () => {
    const response = await service.request({
      action: 'start',
      sourceSessionId: 'old',
      paneId: 'same-pane',
      workspaceId: 'same-workspace',
    });
    const id = response.operations[0].operationId;
    await service.idle(id);
    return id;
  };
  const bind = async (id: string) => {
    bound = true;
    await service.request({ action: 'bind', operationId: id });
    await service.idle(id);
  };
  return {
    cwd,
    filename,
    brief,
    state,
    host,
    service,
    start,
    bind,
    launch,
    sent,
    receipt: () => receipt,
    owner: () => owner,
  };
}

describe('host-owned manager replacement transaction', () => {
  it('projects a safe stale-checkpoint diagnostic and retains ownership before spawning', async () => {
    const f = fixture();
    const send = f.host.send;
    f.host.send = async (id, text) => {
      const result = await send(id, text);
      if (text.startsWith('HOST-OWNED')) fs.appendFileSync(f.brief, '\r\nLate checkpoint edit');
      return result;
    };
    const id = await f.start();
    expect(f.state.views()[0]).toMatchObject({
      operationId: id,
      phase: 'failed',
      committed: false,
      error:
        'Checkpoint files[0] sha256 does not match current file bytes; checkpoint may have changed',
    });
    expect(f.owner()).toBe('old');
    expect(f.host.spawn).not.toHaveBeenCalled();
    expect(f.host.transfer).not.toHaveBeenCalled();
  });
  it('captures a completion already awaiting acknowledgement and does not checkpoint or spawn after that acknowledgement is lost', async () => {
    const f = fixture();
    f.host.inFlightMessages = () => [{ id: 'early-message', text: 'Completion sent before click' }];
    f.host.flushFinishes = async () =>
      f.state.noteInFlightMessage('old', 'early-message', false, 'Acknowledgement lost');
    const id = await f.start();
    expect(f.state.get(id)).toMatchObject({ phase: 'recovery-required', committed: false });
    expect(f.state.get(id).deliveries[0]).toMatchObject({
      text: 'Completion sent before click',
      status: 'uncertain',
    });
    expect(f.host.spawn).not.toHaveBeenCalled();
    expect(f.sent).toEqual([]);
    expect(f.host.pause).toHaveBeenCalledWith('old');
  });
  it('replays no sending receipt after a desktop restart and retains the queued bytes', async () => {
    const f = fixture();
    f.host.inFlightMessages = () => [{ id: 'prior-message', text: 'Prior completion' }];
    let settle!: () => void;
    f.host.flushFinishes = () =>
      new Promise<void>((r) => {
        settle = r;
      });
    const response = await f.service.request({
      action: 'start',
      sourceSessionId: 'old',
      paneId: 'same-pane',
      workspaceId: 'same-workspace',
    });
    const id = response.operations[0].operationId;
    const disk = new ManagerReplacementState(() => f.filename);
    expect(disk.get(id).deliveries[0].status).toBe('sending');
    // Let the original fixture settle as a known ambiguity, without leaving a timer behind.
    f.state.noteInFlightMessage('old', 'prior-message', false, 'Lost acknowledgement');
    settle();
    await f.service.idle(id);
    const restored = new ManagerReplacementService(disk, f.host);
    await restored.initialize();
    expect(disk.get(id).deliveries[0]).toMatchObject({
      text: 'Prior completion',
      status: 'uncertain',
    });
    expect(f.host.spawn).not.toHaveBeenCalled();
    expect(f.sent).toEqual([]);
  });
  it('drains an admitted asynchronous spawn before snapshotting workers and tasks', async () => {
    const f = fixture();
    let release!: () => void;
    let allocated = false;
    const inventory = f.host.inventory;
    const tasks = f.host.tasks;
    f.host.inventory = (id) => [
      ...inventory(id),
      ...(allocated ? [{ sessionId: 'late-worker', cwd: f.cwd, parentSessionId: 'old' }] : []),
    ];
    f.host.tasks = (id) => [...tasks(id), ...(allocated ? ['late-task'] : [])];
    const admitted = f.state.admitted(['old'], async () => {
      await new Promise<void>((r) => {
        release = r;
      });
      allocated = true;
    });
    const response = await f.service.request({
      action: 'start',
      sourceSessionId: 'old',
      paneId: 'same-pane',
      workspaceId: 'same-workspace',
    });
    expect(f.host.spawn).not.toHaveBeenCalled();
    release();
    await admitted;
    await f.service.idle(response.operations[0].operationId);
    expect(f.state.get(response.operations[0].operationId).workerIds).toContain('late-worker');
    expect(f.state.get(response.operations[0].operationId).taskIds).toContain('late-task');
  });
  it('cancellation while successor validation is awaiting cannot cross the transfer boundary', async () => {
    const f = fixture();
    let finish!: () => void;
    let reached!: () => void;
    const entered = new Promise<void>((r) => {
      reached = r;
    });
    f.host.validateSuccessor = async () => {
      reached();
      await new Promise<void>((r) => {
        finish = r;
      });
    };
    const response = await f.service.request({
      action: 'start',
      sourceSessionId: 'old',
      paneId: 'same-pane',
      workspaceId: 'same-workspace',
    });
    const id = response.operations[0].operationId;
    await entered;
    await f.service.request({ action: 'cancel', operationId: id });
    finish();
    await f.service.idle(id);
    expect(f.state.get(id).phase).toBe('cancelled');
    expect(f.host.transfer).not.toHaveBeenCalled();
    expect(f.host.close).not.toHaveBeenCalledWith('old');
  });
  it('times out a stalled spawn while retaining the old owner', async () => {
    const f = fixture();
    f.host.spawn = () => new Promise(() => {});
    const id = await f.start();
    expect(f.state.get(id)).toMatchObject({ phase: 'failed', committed: false });
    expect(f.state.get(id).error).toContain('spawn timed out');
    expect(f.owner()).toBe('old');
  });
  it('retries explicit not-ready rejections, but never an unknown acknowledgement', async () => {
    const f = fixture();
    const send = f.host.send;
    let refusals = 0;
    f.host.send = (id, text) => {
      if (text === 'Successor kickoff' && refusals++ < 2) throw new ManagerDeliveryRejected(404);
      return send(id, text);
    };
    const id = await f.start();
    await f.bind(id);
    expect(f.state.get(id).phase).toBe('complete');
    expect(f.host.pause).not.toHaveBeenCalled();
    expect(f.sent.filter((s) => s.text === 'Successor kickoff')).toHaveLength(1);
  });
  it('keeps an ambiguous queued wake durable without replaying an accepted kickoff', async () => {
    const f = fixture();
    const send = f.host.send;
    f.host.spawn = async () => {
      f.state.holdMessage('old', 'Queued worker result');
    };
    f.host.send = (id, text) => {
      if (text.startsWith('Queued worker result')) throw new Error('Lost wake acknowledgement');
      return send(id, text);
    };
    const id = await f.start();
    await f.bind(id);
    const saved = new ManagerReplacementState(() => f.filename).get(id);
    expect(saved.phase).toBe('recovery-required');
    expect(saved.deliveries.find((d) => d.kind === 'message')).toMatchObject({
      text: expect.stringContaining('Queued worker result'),
      status: 'uncertain',
    });
    await f.service.request({ action: 'reconcile', operationId: id });
    expect(f.sent.filter((s) => s.text === 'Successor kickoff')).toHaveLength(1);
    expect(f.host.close).not.toHaveBeenCalledWith('old');
  });
  it('checkpoints, validates, spawns one parked manager, transfers all ownership, binds same pane, then activates and retires', async () => {
    const f = fixture();
    const id = await f.start();
    const op = f.state.get(id);
    expect(op.phase).toBe('binding');
    expect(op.committed).toBe(true);
    expect(f.host.spawn).toHaveBeenCalledExactlyOnceWith(op.successorSessionId, f.launch);
    expect(f.owner()).toBe(op.successorSessionId);
    expect(op.workerIds).toEqual(['worker', 'pending']);
    expect(op.taskIds).toEqual(['ordinary-task', 'workflow-task']);
    expect(f.sent.map((s) => s.id)).toEqual(['old']);
    expect(f.host.close).not.toHaveBeenCalled();
    await f.bind(id);
    expect(f.state.get(id)).toMatchObject({
      phase: 'complete',
      bound: true,
      retired: true,
      paneId: 'same-pane',
      workspaceId: 'same-workspace',
    });
    expect(f.sent.at(-1)).toEqual({ id: op.successorSessionId, text: 'Successor kickoff' });
    expect(f.host.close).toHaveBeenCalledExactlyOnceWith('old');
    expect(f.state.held('old')).toBeDefined();
    expect(f.state.held(op.successorSessionId)).toBeUndefined();
  });
  it('duplicate clicks and renderer retries converge without spawning a second successor', async () => {
    const f = fixture();
    const [a, b] = await Promise.all([
      f.service.request({
        action: 'start',
        sourceSessionId: 'old',
        paneId: 'same-pane',
        workspaceId: 'same-workspace',
      }),
      f.service.request({
        action: 'start',
        sourceSessionId: 'old',
        paneId: 'same-pane',
        workspaceId: 'same-workspace',
      }),
    ]);
    const id = a.operations[0].operationId;
    expect(b.operations[0].operationId).toBe(id);
    await f.service.idle(id);
    await f.bind(id);
    await f.service.request({
      action: 'start',
      sourceSessionId: 'old',
      paneId: 'same-pane',
      workspaceId: 'same-workspace',
    });
    expect(f.host.spawn).toHaveBeenCalledTimes(1);
  });
  it.each(['preparation', 'spawn', 'identity'])(
    'fails closed at %s without moving ownership or retiring the old manager',
    async (fault) => {
      const f = fixture();
      if (fault === 'preparation') f.host.send = async () => ({ ok: true });
      if (fault === 'spawn')
        f.host.spawn = async () => {
          throw new Error('Spawn failed');
        };
      if (fault === 'identity')
        f.host.validateSuccessor = async () => {
          throw new Error('Wrong cwd/provider/grants');
        };
      const id = await f.start();
      expect(f.state.get(id).phase).toBe('failed');
      expect(f.owner()).toBe('old');
      expect(f.host.transfer).not.toHaveBeenCalled();
      expect(f.host.close).not.toHaveBeenCalledWith('old');
      expect(f.state.held('old')).toBeUndefined();
    },
  );
  it('journals transfer intent and rolls ownership forward after a transfer fault; UI failure never retires the source', async () => {
    const f = fixture();
    const transfer = f.host.transfer;
    f.host.transfer = vi.fn(() => {
      throw new Error('Injected transfer failure');
    });
    const id = await f.start();
    expect(f.state.get(id)).toMatchObject({
      phase: 'recovery-required',
      transferIntent: true,
      committed: false,
    });
    expect(f.host.close).not.toHaveBeenCalledWith('old');
    f.host.transfer = transfer;
    await f.service.request({ action: 'reconcile', operationId: id });
    const failure = await f.service.request({ action: 'bind', operationId: id });
    expect(failure.error).toContain('same pane');
    expect(f.owner()).toBe(f.state.get(id).successorSessionId);
    await f.bind(id);
    expect(f.state.get(id).phase).toBe('complete');
  });
  it('holds completion evidence and message delivery until transfer and pane binding', async () => {
    const f = fixture();
    f.host.spawn = vi.fn(async () => {
      f.state.recordFinish('old', 'worker', 'Finished during spawn', false);
      expect(f.state.holdMessage('old', 'Worker finished', [['worker', 'reply-signature']])).toBe(
        true,
      );
    });
    const id = await f.start();
    expect(f.state.get(id).deliveries.find((d) => d.kind === 'message')?.status).toBe('pending');
    expect(f.state.signature('worker')).toBe('reply-signature');
    expect(f.state.get(id).finishes).toEqual({});
    await f.bind(id);
    expect(f.sent.slice(1).map((s) => s.text)).toEqual([
      'Successor kickoff',
      expect.stringContaining('Worker finished'),
    ]);
    expect(f.sent.at(-1)?.text).toContain(
      `Current manager/parentSessionId is ${f.state.get(id).successorSessionId}`,
    );
  });
  it('records ambiguous kickoff acknowledgement, pauses, never replays automatically, and permits only explicit retry', async () => {
    const f = fixture();
    const send = f.host.send;
    f.host.send = async (id, text) => {
      if (text === 'Successor kickoff') throw new Error('Acknowledgement lost');
      return send(id, text);
    };
    const id = await f.start();
    await f.bind(id);
    const op = f.state.get(id);
    const kickoff = op.deliveries.find((d) => d.kind === 'kickoff')!;
    expect(op.phase).toBe('recovery-required');
    expect(kickoff.status).toBe('uncertain');
    expect(f.host.close).not.toHaveBeenCalledWith('old');
    expect(f.host.pause).toHaveBeenCalledWith(op.successorSessionId);
    await f.service.request({ action: 'reconcile', operationId: id });
    expect(f.state.get(id).phase).toBe('recovery-required');
    f.host.send = send;
    await f.service.request({
      action: 'resolve-delivery',
      operationId: id,
      deliveryId: kickoff.id,
      resolution: 'retry',
      acknowledgeDuplicateRisk: true,
    });
    await f.service.idle(id);
    expect(f.state.get(id).phase).toBe('complete');
  });
  it('recovers a process interrupted after ownership commit without duplicate spawn or replay of sending evidence', async () => {
    const f = fixture();
    const id = await f.start();
    const delivery = f.state.delivery(id, 'kickoff', 'Unacknowledged kickoff');
    f.state.change(id, (o) => {
      o.deliveries.find((d) => d.id === delivery)!.status = 'sending';
    });
    const recoveredState = new ManagerReplacementState(() => f.filename);
    const recovered = new ManagerReplacementService(recoveredState, f.host);
    await recovered.initialize();
    expect(recoveredState.get(id)).toMatchObject({ committed: true, phase: 'recovery-required' });
    expect(recoveredState.get(id).deliveries.at(-1)?.status).toBe('uncertain');
    expect(f.host.spawn).toHaveBeenCalledTimes(1);
    expect(f.sent).toHaveLength(1);
  });
  it('does not use generic idle, prose, or a previous handoff artifact as checkpoint completion', async () => {
    const f = fixture();
    const id = await f.start();
    const op = f.state.get(id);
    expect(() => validateManagerArtifact(op, 'Handoff complete')).toThrow('receipt');
    const artifact = JSON.parse(fs.readFileSync(op.artifactPath, 'utf8'));
    artifact.operationId = 'another-operation';
    fs.writeFileSync(op.artifactPath, JSON.stringify(artifact));
    expect(() => validateManagerArtifact(op, f.receipt())).toThrow('identity');
  });
  it('refuses remote or unowned source before journaling or spawn', async () => {
    const f = fixture();
    f.host.source = () => {
      throw new Error('Remote and headless replacement unavailable');
    };
    const result = await f.service.request({
      action: 'start',
      sourceSessionId: 'remote',
      paneId: 'viewer',
      workspaceId: 'remote-workspace',
    });
    expect(result.error).toContain('unavailable');
    expect(f.state.records()).toEqual([]);
    expect(f.host.spawn).not.toHaveBeenCalled();
  });
});
