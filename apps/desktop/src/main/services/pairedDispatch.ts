import { randomBytes, randomUUID } from 'crypto';
import path from 'path';
import { getConfigDir } from './configService';
import {
  pairedWorkerConnection as connection,
  pairedDestinationKey,
} from './pairedWorkerConnection';
import { getPairedWorkerTarget } from './remoteServer';
import {
  remoteDispatchRegistry as registry,
  sanitizeRemoteEntry,
  DISPATCH_PROTOCOL,
} from './remoteDispatchRegistry';
import { claudeSessionStore } from './claudeSessionStore';
import { managerReplacementState } from './managerReplacementState';
import { dispatchHistoryStore } from './dispatchHistoryStore';
import { readStructuredResult, buildResultContract } from '../shared/structuredResult';
import { readWorkerEscalation } from '../shared/workerEscalation';
import { buildFleetMessage } from '../shared/fleetMessages';
import { workflowWakeInstructions } from './fleetWorkflowRuntime';
import { claudemonSessionClient } from './claudemonSessionClient';
import { renderDispatchTemplate } from '../lib/dispatchTemplate';
import { prepareTaskHandoff, importTaskHandoffResult, type TaskSource } from './taskHandoff';
import { fleetReviewStore, reviewAllocation } from './fleetReviewStore';

function projectPairedSnapshot(
  record: import('./remoteDispatchRegistry').RemoteDispatchRecord,
  snapshot: import('./claudeSessionStore').RemoteSnapshotWire,
): void {
  if (!record.localSessionId) return;
  claudeSessionStore.upsertRemoteSession('@paired', {
    ...snapshot,
    sessionId: record.localSessionId,
    parentSessionId: record.ownerSessionId,
    isWakeTarget: false,
  });
  const state = String(snapshot.ambientState ?? '');
  try {
    dispatchHistoryStore.observeRemote(
      record.localSessionId,
      snapshot.status === 'ended'
        ? 'ended'
        : state === 'idle'
          ? 'idle'
          : ['waiting_approval', 'waiting_input'].includes(state)
            ? 'needs-decision'
            : 'running',
      record.state === 'done',
    );
  } catch {
    // Historical task retention may have expired; the remote card remains an
    // observation and never creates a replacement task or a local fs grant.
  }
}

let started = false;
let deliveries = Promise.resolve();

export function startPairedDispatch(): void {
  if (started) return;
  registry.start(path.join(getConfigDir(), 'remote-dispatches.json'), (id) => {
    const target = managerReplacementState.automaticWakeTarget(id);
    const manager = claudeSessionStore.getSnapshot(target);
    return manager?.isWakeTarget && !manager.hub && manager.status !== 'ended' ? target : null;
  });
  started = true;
  connection.onDisconnected = () => claudeSessionStore.markHubPeerOffline('@paired');
  registry.onReparent = () => connection.onConnected();
  connection.onConnected = () => {
    for (const record of registry.list()) {
      if (record.peer === pairedDestinationKey() && record.state === 'done') {
        void connection
          .call('agents.dispatchReplay', {
            dispatchId: record.dispatchId,
            ackedSeq: record.ackedSeq,
          })
          .catch(() => {});
      }
    }
    for (const record of registry.openForPeer(pairedDestinationKey())) {
      void connection
        .call<{ state?: string; dispatchId?: string }>('agents.dispatchReplay', {
          dispatchId: record.dispatchId,
        })
        .then((reply) => {
          if (
            reply?.state === 'unknown' &&
            reply.dispatchId === record.dispatchId &&
            record.peer === pairedDestinationKey()
          ) {
            registry.markLost(
              record.dispatchId,
              'The paired server no longer has a record of this dispatch. Worker outcome is unknown; reconcile the remote worker before retrying. Do not repeat the spawn.',
            );
          }
        })
        .catch(() => {});
      if (record.sessionId && record.localSessionId)
        void connection
          .call<import('./claudeSessionStore').RemoteSnapshotWire>('sessions.snapshot', {
            sessionId: record.sessionId,
          })
          .then((snapshot) => projectPairedSnapshot(record, snapshot))
          .catch(() => {});
    }
  };
  connection.onEvent = (event) => {
    if (event.type === 'agent.snapshot') {
      const snapshot = event.data as import('./claudeSessionStore').RemoteSnapshotWire;
      const record = registry
        .list()
        .find((r) => r.peer === pairedDestinationKey() && r.sessionId === snapshot?.sessionId);
      if (record) projectPairedSnapshot(record, snapshot);
      return;
    }
    if (event.type !== 'agent.dispatch.update') return;
    // Serial delivery keeps duplicate callbacks from both passing the seq gate.
    deliveries = deliveries.then(() => deliverPairedUpdate(event.data)).catch(() => {});
  };
  if (getPairedWorkerTarget()) void connection.connect().catch(() => {});
}

async function deliverPairedUpdate(data: unknown): Promise<void> {
  const accepted = registry.accept(pairedDestinationKey(), data);
  if (!accepted.ok) return;
  const { record, update, parentSessionId } = accepted;
  if (!record.localSessionId) return;
  registry.retainEvidence(record.dispatchId, update);
  const entry = sanitizeRemoteEntry(update.entry);
  entry.sessionId = record.localSessionId;
  let checkpointRequired = false;
  if (update.final && record.handoff) {
    let result;
    try {
      result = await importTaskHandoffResult(record.dispatchId, record.handoff.binding);
    } catch (error) {
      dispatchHistoryStore.observeHandoff(entry.sessionId, {
        state: 'blocked',
        note: `Output custody pending: ${String(error)}`,
      });
      throw error; // Retain the update; no acknowledgment before byte custody.
    }
    registry.setHandoff(record.dispatchId, {
      ...record.handoff,
      state: result.state,
      reviewCwd: result.state === 'received' ? result.allocation : undefined,
    });
    if (result.state === 'needs-checkpoint') {
      checkpointRequired = true;
      entry.needsDecision = true;
      entry.failed =
        'Checkpoint required: execution workspace has uncommitted changes; output is retained.';
    } else {
      entry.note =
        'Code and required artifacts verified locally. Reported test results remain worker claims.';
      if (result.allocation && result.result && record.handoff.sourceCwd) {
        const branch = `wks/handoff-${record.dispatchId}-result`;
        const allocation = await reviewAllocation(
          path.join(path.dirname(result.allocation), 'result-git'),
          result.allocation,
          branch,
        );
        fleetReviewStore.register(parentSessionId, entry.sessionId, {
          ...allocation,
          projectRoot: record.handoff.sourceCwd,
          baseCommit: result.plan.input.commit,
        });
        entry.reviewEvidenceId = await fleetReviewStore.capture(
          parentSessionId,
          entry.sessionId,
          'turn-ended',
        );
      }
    }
    dispatchHistoryStore.observeHandoff(entry.sessionId, {
      state: checkpointRequired ? 'needs-checkpoint' : 'received',
      base: result.plan.input.commit,
      head: result.result?.commit,
      reviewCwd: result.state === 'received' ? result.allocation : undefined,
      artifacts: result.result?.entries,
      artifactTask: record.dispatchId,
    });
  }
  // No remote snapshot or parent id is trusted to select a local recipient.
  const reply = entry.fullReply ?? entry.lastReply ?? '';
  if (update.final) {
    const escalation = readWorkerEscalation(reply);
    if (escalation?.json) entry.escalation = escalation.json;
    if (escalation?.error) entry.escalationError = escalation.error;
    if (record.resultSchema && !entry.escalation) {
      const result = readStructuredResult(reply, record.resultSchema);
      if (result.json) entry.result = result.json;
      if (result.error) entry.resultError = result.error;
    }
    dispatchHistoryStore.validated(
      entry.sessionId,
      checkpointRequired
        ? 'invalid'
        : entry.escalation
          ? 'escalated'
          : entry.result
            ? 'valid'
            : entry.resultError
              ? 'invalid'
              : 'absent',
      entry.reviewEvidenceId,
      entry.result ? JSON.parse(entry.result) : entry.escalation,
    );
  }
  dispatchHistoryStore.observeRemote(
    entry.sessionId,
    update.kind === 'blocked'
      ? 'needs-decision'
      : update.final
        ? entry.stopped
          ? 'ended'
          : 'idle'
        : 'running',
    update.final,
  );
  const text =
    buildFleetMessage(update.kind, [entry]) + workflowWakeInstructions([entry.sessionId]);
  const signatures: Array<[string, string]> = [
    [entry.sessionId, `paired:${record.dispatchId}:${update.seq}`],
  ];
  if (managerReplacementState.signature(entry.sessionId) !== signatures[0][1]) {
    registry.beginDelivery(record.dispatchId, update.seq);
    const response = await claudemonSessionClient.message(parentSessionId, text, signatures);
    if (!response.ok) throw new Error('Remote result wake was not accepted');
    managerReplacementState.recordSignature(entry.sessionId, signatures[0][1]);
  }
  registry.acknowledge(record.dispatchId, update);
  if (update.final) {
    void connection
      .call('agents.dispatchReplay', { dispatchId: record.dispatchId, ackedSeq: update.seq })
      .catch(() => {});
  }
}

type Admission = Parameters<typeof dispatchHistoryStore.accept>[0];

export async function spawnPairedWorker(
  p: Record<string, unknown>,
  admission: Omit<Admission, 'sessionId' | 'executionCwd'>,
  templateBody?: string,
  resultSchema?: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  startPairedDispatch();
  if (p.executionTarget !== 'paired' || !getPairedWorkerTarget())
    throw new Error('Paired worker target is not enabled');
  const owner = admission.owner;
  if (
    !owner?.isWakeTarget ||
    owner.hub ||
    owner.status === 'ended' ||
    p.parentSessionId !== owner.sessionId
  )
    throw new Error('Paired dispatch requires the authenticated live local manager');
  if (
    p.manager ||
    p.resumeSessionId ||
    p.retrySourceSessionId ||
    p.profileId ||
    p.mcpItemIds ||
    p.pluginTools ||
    p.launchIntegrationId
  )
    throw new Error(
      'Paired workers require a fresh session without local profile or process configuration',
    );
  const capabilities = await connection.call<{
    handoff?: { version: number; receiptVersion: number; transport: string; chunkBytes: number };
    protocol: number;
    executes: boolean;
    cwds: Array<{ path: string }>;
    providers: Array<{ provider: string; found: boolean; authenticated: boolean | null }>;
  }>('fleet.dispatchCapabilities');
  if (capabilities.protocol !== DISPATCH_PROTOCOL || !capabilities.executes)
    throw new Error('Paired worker dispatch unsupported; upgrade the older endpoint');
  const taskSource = p.taskSource as TaskSource | undefined;
  if (
    taskSource &&
    (capabilities.handoff?.version !== 1 ||
      capabilities.handoff.receiptVersion !== 1 ||
      capabilities.handoff.transport !== 'git-remote' ||
      capabilities.handoff.chunkBytes !== 256 * 1024)
  )
    throw new Error(
      'Exact workspace handoff is unsupported by this target; upgrade before dispatch',
    );
  if (!capabilities.cwds?.some((c) => c.path === p.remoteCwd))
    throw new Error('Choose an actual remote cwd from list_dispatch_targets');
  if (
    !capabilities.providers?.some(
      (provider) =>
        provider.provider === p.provider && provider.found && provider.authenticated === true,
    )
  )
    throw new Error('Selected provider is not authenticated on the remote host');
  const dispatchId = randomBytes(32).toString('hex');
  const localSessionId = `paired:${randomUUID()}`;
  const record = registry.open({
    dispatchId,
    peer: pairedDestinationKey(),
    ownerSessionId: owner.sessionId,
    localSessionId,
    resultSchema,
    cwd: p.remoteCwd,
    provider: p.provider,
    model: p.model,
    label: p.label,
  });
  if (!record) throw new Error('Could not persist paired dispatch admission');
  const preAdmission = taskSource
    ? dispatchHistoryStore.accept({
        ...admission,
        sessionId: localSessionId,
        executionCwd: String(p.remoteCwd),
        provider: String(p.provider),
        requestedProvider: String(p.provider),
        requestedModel: typeof p.model === 'string' ? p.model : undefined,
        worktree: { requested: true, allocated: false, fallback: false },
        executionTarget: 'paired',
        executionHost:
          getPairedWorkerTarget()!.displayName || new URL(getPairedWorkerTarget()!.httpUrl).host,
      })
    : undefined;
  let handoff: import('./taskHandoff').HandoffReceiptSelector | undefined;
  if (taskSource) {
    dispatchHistoryStore.observeHandoff(localSessionId, { state: 'preparing' });
    registry.setHandoff(dispatchId, {
      binding: taskSource.binding,
      state: 'preparing',
      sourceCwd: String(p.cwd),
    });
    let prepared;
    try {
      prepared = await prepareTaskHandoff(
        dispatchId,
        taskSource,
        String(p.provider),
        String(p.cwd),
      );
    } catch (error) {
      dispatchHistoryStore.observeHandoff(localSessionId, {
        state: 'blocked',
        note: String(error),
      });
      throw error;
    }
    handoff = {
      version: 1,
      binding: taskSource.binding,
      digest: prepared.digest,
      allocationId: prepared.allocationId!,
    };
    registry.setHandoff(dispatchId, { ...handoff, state: 'prepared', sourceCwd: String(p.cwd) });
  }
  const remoteOrigin = { protocol: DISPATCH_PROTOCOL, dispatchId };
  const prepared = await connection.call<{
    cwd: string;
    repo: string;
    worktree: boolean;
    branch?: string;
  }>('agents.dispatchPrepare', {
    remoteOrigin,
    ...(handoff ? { handoff } : {}),
    cwd: p.remoteCwd,
    provider: p.provider,
    worktree: handoff ? true : p.worktree === true,
  });
  const worktree = {
    requested: p.worktree === true,
    allocated: prepared.worktree,
    fallback: false,
    branch: prepared.branch,
  };
  if (p.worktree && !prepared.worktree) throw new Error('Remote isolated worktree required');
  if (taskSource)
    dispatchHistoryStore.preparePairedHandoff(localSessionId, prepared.cwd, prepared.branch ?? '');
  // Book the local workflow attempt before starting the remote process. A lost
  // reply leaves this attempt pending and prevents blindly repeating the step.
  const ids =
    preAdmission ??
    dispatchHistoryStore.accept({
      ...admission,
      sessionId: localSessionId,
      executionCwd: prepared.cwd,
      provider: String(p.provider),
      requestedProvider: String(p.provider),
      requestedModel: typeof p.model === 'string' ? p.model : undefined,
      worktree,
      executionTarget: 'paired',
      executionHost:
        getPairedWorkerTarget()!.displayName || new URL(getPairedWorkerTarget()!.httpUrl).host,
    });
  let message = templateBody
    ? renderDispatchTemplate(templateBody, (p.templateParams ?? {}) as Record<string, string>, {
        cwd: prepared.cwd,
        projectCwd: prepared.repo,
      })
    : p.message;
  if (resultSchema) message = `${message ?? ''}\n\n${buildResultContract(resultSchema)}`;
  if (taskSource) {
    const folder = `.workspacer/handoffs/${dispatchId}`;
    message = `${message ?? ''}\n\nTask evidence folder: ${folder}. Selected inputs: ${taskSource.artifacts.map((a) => a.name).join(', ') || 'none'}. Required output artifacts: ${taskSource.outputs.map((a) => a.name).join(', ') || 'none'}. The workspace host transfers code and artifacts; keep this evidence folder out of code commits.`;
    dispatchHistoryStore.observeHandoff(localSessionId, { state: 'running' });
  }
  // Allowlist supported execution metadata. All origin workflow/request and
  // grant fields stay local. The remote token and remote routing ceiling win.
  const wire: Record<string, unknown> = {
    remoteOrigin,
    ...(handoff ? { handoff } : {}),
    cwd: prepared.cwd,
    provider: p.provider,
    transport: 'stream',
    message,
    skipPermissions: false,
    toolScope: p.toolScope,
    label: p.label,
    model: p.model,
    modelIdentity: p.modelIdentity,
    contextWindow: p.contextWindow,
    effort: p.effort,
    role: p.role,
    capability: p.capability,
    decisionId: p.decisionId,
  };
  try {
    const result = await connection.call<{ sessionId?: string; messageQueued?: boolean }>(
      'agents.spawn',
      wire,
    );
    if (!result.sessionId || result.messageQueued !== true)
      throw new Error(
        'Remote admission or initial delivery is uncertain; do not repeat spawn or send the initial message',
      );
    registry.attachSession(dispatchId, result.sessionId);
    const snapshot = await connection
      .call<import('./claudeSessionStore').RemoteSnapshotWire>('sessions.snapshot', {
        sessionId: result.sessionId,
      })
      .catch(() => ({ cwd: prepared.cwd, status: 'starting' as const }));
    claudeSessionStore.upsertRemoteSession('@paired', {
      ...snapshot,
      sessionId: localSessionId,
      parentSessionId: owner.sessionId,
      isWakeTarget: false,
    });
    return {
      ...ids,
      sessionId: localSessionId,
      remoteSessionId: result.sessionId,
      executionTarget: 'paired',
      executionCwd: prepared.cwd,
      worktreeResult: worktree,
      messageQueued: true,
    };
  } catch (err) {
    registry.fail(
      dispatchId,
      'Remote admission unknown; reconcile this dispatch, never blindly repeat it',
    );
    throw err;
  }
}

export async function selectPairedModel(raw: unknown): Promise<unknown> {
  const p = (raw ?? {}) as Record<string, unknown>;
  if (!getPairedWorkerTarget()) throw new Error('Paired worker target is not enabled');
  const caps = await connection.call<{
    protocol: number;
    executes: boolean;
    cwds: Array<{ path: string }>;
    providers: Array<{ provider: string; authenticated: boolean | null; found: boolean }>;
  }>('fleet.dispatchCapabilities');
  if (caps.protocol !== DISPATCH_PROTOCOL || !caps.executes)
    throw new Error('Paired dispatch unsupported; upgrade the older endpoint');
  if (!caps.cwds.some((c) => c.path === p.cwd))
    throw new Error('Select a remote repository returned by list_dispatch_targets');
  if (!caps.providers.some((v) => v.provider === p.provider && v.found && v.authenticated === true))
    throw new Error('Explicitly choose a provider authenticated on the remote host');
  const { role, provider, cwd, difficulty, risk, decisionDensity } = p;
  return connection.call('routing.select', {
    role,
    provider,
    cwd,
    difficulty,
    risk,
    decisionDensity,
    previousProvider: p.previousProvider,
    requireIndependentFamily: p.requireIndependentFamily,
    profile: p.profile,
    forecastDemandBeforeResetPct: p.forecastDemandBeforeResetPct,
    expectedWork: p.expectedWork,
  });
}
