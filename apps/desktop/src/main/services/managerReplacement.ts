import { ownsHubCapability } from './hubClient';
import { readAgentRuntimeStatus } from './agentRuntimeStatus';
import { getRemoteServer } from './remoteServer';
import { isHubAdopted } from './hubDaemon';
import type {
  ManagerReplacementRequest,
  ManagerReplacementResponse,
} from '../shared/managerReplacement';
import { thresholdWatcher } from './thresholdWatcher';
import { readSelectionSlice } from '../shared/canonicalSelection';
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { dispatchHistoryStore } from './dispatchHistoryStore';
import { supervisorNudge } from './supervisorNudge';
import { spawnManagedAgent } from './managedSpawn';
import { managerReplacementState, type ManagerLaunch } from './managerReplacementState';
import { ManagerReplacementService } from './managerReplacementService';
import { sessionFacadeGrantFingerprint, revokeSessionFacadeTokens } from './remoteTokens';
import { managerLaunchConfiguration } from './managerLaunchConfiguration';
import {
  MANAGER_REPLACEMENT_UNAVAILABLE,
  ManagerReplacementUnavailable,
} from '../shared/managerReplacement';
import { buildManagerKickoff } from '../shared/managerDoctrine';
import { managerFullAccessFromConfig } from './fullAccessGrants';
import { normalizeModelSelection } from '../shared/modelContextWindows';

function assertLocalHost(): void {
  if (
    getRemoteServer() ||
    isHubAdopted() ||
    !['agents.spawn', 'agents.reparent', 'agents.sendMessage', 'fleetWorkflows.request'].every(
      ownsHubCapability,
    )
  )
    throw new ManagerReplacementUnavailable(MANAGER_REPLACEMENT_UNAVAILABLE);
}
function source(id: string, paneId?: string): ManagerLaunch {
  assertLocalHost();
  const session = claudeSessionStore.getSnapshot(id);
  const recorded = managerReplacementState.launch(id);
  const grants = sessionFacadeGrantFingerprint(id);
  if (
    !session ||
    session.status === 'ended' ||
    session.hub ||
    !session.isWakeTarget ||
    !recorded ||
    !grants ||
    session.transport !== 'stream' ||
    (paneId && claudemonSessionClient.attachedSession(paneId) !== id)
  )
    throw new ManagerReplacementUnavailable(MANAGER_REPLACEMENT_UNAVAILABLE);
  if (recorded.options.launchIntegrationId)
    throw new ManagerReplacementUnavailable(
      'Automatic manager replacement is unavailable for custom launch integrations',
    );
  if (recorded.configuration !== managerLaunchConfiguration(recorded.options))
    throw new Error(
      'Manager profile configuration changed. Automatic replacement cannot reproduce this launch.',
    );
  const settings = session.settings;
  const permissionMode =
    session.livePermissionMode ?? settings?.permissionMode ?? recorded.options.permissionMode;
  const model =
    session.requestedSelection?.model ??
    settings?.model ??
    recorded.options.model ??
    session.usage?.model;
  if (!model)
    throw new Error(
      'Manager model is not recorded yet. Wait for its first configured turn before handoff.',
    );
  const selection = normalizeModelSelection(
    model,
    session.requestedSelection
      ? session.requestedSelection.contextWindow
      : settings?.contextWindow !== undefined
        ? settings.contextWindow
        : recorded.options.contextWindow,
  );
  return {
    ...recorded,
    grants,
    options: {
      ...recorded.options,
      cwd: session.cwd,
      label: session.label ?? recorded.options.label,
      parentSessionId: session.parentSessionId,
      provider: session.provider as ManagerLaunch['options']['provider'],
      manager: true,
      toolScope: 'operator',
      model: selection.model,
      modelIdentity: selection.model,
      contextWindow: selection.contextWindow,
      effort:
        session.liveEffort ??
        session.statusLine?.effort ??
        settings?.effort ??
        settings?.defaultEffort ??
        recorded.options.effort,
      permissionMode,
      skipPermissions: permissionMode === 'bypassPermissions' || permissionMode === 'yolo',
    },
  };
}

async function validateSuccessor(id: string, launch: ManagerLaunch): Promise<void> {
  assertLocalHost();
  if ((await readAgentRuntimeStatus()).facade !== 'ready')
    throw new ManagerReplacementUnavailable(
      'Manager action tools are not ready; ownership is retained until the facade reconnects',
    );
  const wire = await claudemonSessionClient.getSession(id);
  const s = claudeSessionStore.getSnapshot(id);
  const operation = managerReplacementState.related(id);
  if (operation && !operation.transferIntent && (!wire || wire.user_prompts !== 0))
    throw new ManagerReplacementUnavailable(
      'The successor must report zero user prompts before transfer; a parked session or newer host is required',
    );
  if (
    !wire ||
    wire.session_id !== id ||
    wire.mode === 'stopped' ||
    wire.cwd !== launch.options.cwd ||
    wire.provider !== launch.options.provider ||
    !s?.isWakeTarget ||
    s.status === 'ended' ||
    s.hub ||
    sessionFacadeGrantFingerprint(id) !== launch.grants ||
    managerLaunchConfiguration(launch.options) !== launch.configuration
  )
    throw new Error(
      'Successor identity, liveness, cwd, provider or operator grants do not match the source',
    );
  const actual = managerReplacementState.launch(id)?.options;
  if (
    !actual ||
    actual.effort !== launch.options.effort ||
    actual.permissionMode !== launch.options.permissionMode ||
    JSON.stringify(normalizeModelSelection(actual.model!, actual.contextWindow)) !==
      JSON.stringify(normalizeModelSelection(launch.options.model!, launch.options.contextWindow))
  )
    throw new Error('Successor model, context or effort did not reach the manager spawn machinery');
}

export const managerReplacementService = new ManagerReplacementService(managerReplacementState, {
  source,
  inventory: (id) => claudeSessionStore.replacementInventory(id),
  tasks: (id) =>
    dispatchHistoryStore
      .list()
      .filter((t) => t.ownerSessionId === id)
      .map((t) => t.taskId),
  projects: (id) => [
    ...new Set(
      dispatchHistoryStore
        .list()
        .filter((t) => t.ownerSessionId === id)
        .map((t) => t.projectCwd),
    ),
  ],
  readyForTransfer: (id) =>
    !dispatchHistoryStore.list().some((t) => t.ownerSessionId === id && t.dispatchReservation),
  signatures: (ids) => supervisorNudge.replacementSignatures(ids),
  inFlightMessages: (id) => claudemonSessionClient.inFlightMessages?.(id) ?? [],
  finishes: (id) => supervisorNudge.replacementFinishes(id),
  receipt: (id) =>
    [...(claudeSessionStore.getSnapshot(id)?.conversation ?? [])]
      .reverse()
      .find((t) => t.role === 'assistant')?.content ?? '',
  settled: (id) => {
    const s = claudeSessionStore.getSnapshot(id);
    return (
      !!s &&
      s.status !== 'ended' &&
      s.ambientState === 'idle' &&
      !s.pendingApproval &&
      !s.pendingQuestions &&
      !s.activeToolCalls?.length
    );
  },
  async spawn(id, launch) {
    const result = await spawnManagedAgent({
      ...launch.options,
      replacementSessionId: id,
      firstMessage: undefined,
      resumeSessionId: undefined,
    });
    if (result !== id) throw new Error('Daemon returned the wrong successor identity');
  },
  validateSuccessor,
  transfer(oldId, newId, operationId) {
    claudeSessionStore.reparentChildren(oldId, newId, operationId);
    thresholdWatcher.reassignWatcher(oldId, newId);
  },
  async restore(metadata) {
    for (const record of [...metadata].sort(
      (a, b) => Number(!!b.isWakeTarget) - Number(!!a.isWakeTarget),
    )) {
      const m = managerReplacementState.metadata(record.sessionId) ?? record;
      const launch = m.isWakeTarget ? managerReplacementState.launch(m.sessionId) : undefined;
      if (launch)
        m.settings = {
          ...m.settings,
          model: launch.options.model,
          contextWindow: launch.options.contextWindow,
          effort: launch.options.effort,
          permissionMode: launch.options.permissionMode,
        };
      claudeSessionStore.restoreReplacementMetadata(m);
      const wire = await claudemonSessionClient.getSession(m.sessionId);
      if (!wire) continue; // Pending metadata remains pending; never invent a process.
      if (wire.cwd !== m.cwd && m.isWakeTarget)
        throw new Error('Restored manager cwd does not match the journal');
      claudeSessionStore.ensureManagedSession(m.sessionId, wire.cwd);
      claudeSessionStore.applyManagedMode(m.sessionId, wire.mode, {
        provider: wire.provider,
        transport: wire.transport,
        pending: wire.pending,
        selection: readSelectionSlice(wire),
        backgroundTasks: wire.background_tasks,
        subagents: wire.subagents,
      });
      const conversation = await claudemonSessionClient.getConversation(m.sessionId);
      claudeSessionStore.applyConversationDelta({
        session_id: m.sessionId,
        seq: conversation.seq,
        reset: true,
        items: conversation.items as never,
      });
      if (wire.mode === 'stopped')
        claudeSessionStore.handleHookEvent({
          hook_event_name: 'SessionEnd',
          session_id: m.sessionId,
          cwd: wire.cwd,
        });
    }
  },
  bound: (paneId, id) => claudemonSessionClient.attachedSession(paneId) === id,
  send: (id, text) => claudemonSessionClient.messageDirect(id, text),
  pause: (id) => claudemonSessionClient.signal(id, 'SIGINT'),
  async close(id) {
    const wire = await claudemonSessionClient.getSession(id);
    if (wire && wire.mode !== 'stopped') await claudemonSessionClient.signal(id, 'SIGTERM');
    revokeSessionFacadeTokens(id);
  },
  kickoff: (op) =>
    buildManagerKickoff(
      `HOST-OWNED MANAGER HANDOFF ${op.operationId}. Your fresh manager session is ${op.successorSessionId}; predecessor ${op.sourceSessionId} is audit history only. ` +
        `The host has committed worker AND task ownership and replaced the same pane. Do not adopt workers, resume or terminate the predecessor, or use any shared handoff.md. ` +
        `Use this validated handoff (also retained at ${op.sealedArtifactPath ?? op.artifactPath}, SHA-256 ${op.artifactHash}). Preserve pending decisions; take the stated next action within existing authority.\n${op.artifact}`,
      managerFullAccessFromConfig(),
    ),
  flushFinishes: async (ids) => {
    await supervisorNudge.flushReplacementFinishes(ids);
    await claudemonSessionClient.waitForMessages?.(ids);
  },
  recoverFinishes(op) {
    const evidenceById = { ...op.finishes };
    if (op.phase !== 'complete')
      for (const id of op.workerIds) {
        const s = claudeSessionStore.getSnapshot(id);
        if (
          !s ||
          (s.status !== 'ended' && s.ambientState !== 'idle') ||
          !s.conversation.some((t) => t.role === 'user')
        )
          continue;
        const reply =
          [...s.conversation].reverse().find((t) => t.role === 'assistant')?.content ?? '';
        if (reply || s.status === 'ended')
          evidenceById[id] = { reply, stopped: s.status === 'ended' };
      }
    for (const [id, evidence] of Object.entries(evidenceById)) {
      const snapshot = claudeSessionStore.getSnapshot(id);
      if (!snapshot) continue; // Evidence stays in the journal for inspection.
      supervisorNudge.onFinished(
        snapshot,
        op.committed ? op.successorSessionId : op.sourceSessionId,
        evidence.reply,
      );
    }
  },
});

let hostReady = false;
/** Called only after the owned local hub/facade is ready; remote/headless
 * modes never project or act on a desktop's saved operation metadata. */
export async function enableManagerReplacementHost(): Promise<void> {
  assertLocalHost();
  hostReady = true;
  await managerReplacementService.initialize();
}
export async function requestManagerReplacement(
  request: ManagerReplacementRequest,
): Promise<ManagerReplacementResponse> {
  try {
    assertLocalHost();
    if (!hostReady || request.action === 'start') {
      const runtime = await readAgentRuntimeStatus();
      if (
        runtime.claudemon !== 'ready' ||
        runtime.hub !== 'ready' ||
        (request.action === 'start' && runtime.facade !== 'ready')
      )
        return {
          available: false,
          operations: [],
          error:
            'Local manager handoff is waiting for the owned desktop hub, daemon and action tools.',
        };
      if (!hostReady) await enableManagerReplacementHost();
    }
    return managerReplacementService.request(request);
  } catch (error) {
    return {
      available: false,
      operations: [],
      error: error instanceof Error ? error.message : String(error),
    };
  }
}
