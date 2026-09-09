import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { dispatchHistoryStore } from './dispatchHistoryStore';
import { supervisorNudge } from './supervisorNudge';
import { spawnManagedAgent } from './managedSpawn';
import { managerReplacementState, type ManagerLaunch } from './managerReplacementState';
import { ManagerReplacementService } from './managerReplacementService';
import { sessionFacadeGrantFingerprint } from './remoteTokens';
import { managerLaunchConfiguration } from './managerLaunchConfiguration';
import { MANAGER_REPLACEMENT_UNAVAILABLE } from '../shared/managerReplacement';
import { buildManagerKickoff } from '../shared/managerDoctrine';
import { managerFullAccessFromConfig } from './fullAccessGrants';
import { normalizeModelSelection } from '../shared/modelContextWindows';

function source(id: string, paneId?: string): ManagerLaunch {
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
    throw new Error(MANAGER_REPLACEMENT_UNAVAILABLE);
  if (recorded.options.launchIntegrationId)
    throw new Error('Automatic manager replacement is unavailable for custom launch integrations');
  if (recorded.configuration !== managerLaunchConfiguration(recorded.options))
    throw new Error(
      'Manager profile configuration changed. Automatic replacement cannot reproduce this launch.',
    );
  const settings = session.settings;
  const permissionMode =
    session.livePermissionMode ?? settings?.permissionMode ?? recorded.options.permissionMode;
  const model = session.requestedSelection?.model ?? settings?.model ?? recorded.options.model;
  if (!model)
    throw new Error(
      'Manager model is not recorded yet. Wait for its first configured turn before handoff.',
    );
  const selection = normalizeModelSelection(
    model,
    session.requestedSelection?.contextWindow ??
      settings?.contextWindow ??
      recorded.options.contextWindow,
  );
  return {
    ...recorded,
    grants,
    options: {
      ...recorded.options,
      cwd: session.cwd,
      provider: session.provider as ManagerLaunch['options']['provider'],
      manager: true,
      toolScope: 'operator',
      model: selection.model,
      modelIdentity: selection.model,
      contextWindow: selection.contextWindow,
      effort:
        session.liveEffort ??
        settings?.effort ??
        settings?.defaultEffort ??
        recorded.options.effort,
      permissionMode,
      skipPermissions: permissionMode === 'bypassPermissions' || permissionMode === 'yolo',
    },
  };
}

async function validateSuccessor(id: string, launch: ManagerLaunch): Promise<void> {
  const wire = await claudemonSessionClient.getSession(id);
  const s = claudeSessionStore.getSnapshot(id);
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
  signatures: (ids) => supervisorNudge.replacementSignatures(ids),
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
  },
  async restore(metadata) {
    for (const m of metadata) {
      claudeSessionStore.restoreReplacementMetadata(m);
      const wire = await claudemonSessionClient.getSession(m.sessionId);
      if (!wire) continue; // Pending metadata remains pending; never invent a process.
      if (wire.cwd !== m.cwd && m.isWakeTarget)
        throw new Error('Restored manager cwd does not match the journal');
      claudeSessionStore.ensureManagedSession(m.sessionId, wire.cwd);
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
  },
  kickoff: (op) =>
    buildManagerKickoff(
      `HOST-OWNED MANAGER HANDOFF ${op.operationId}. Your fresh manager session is ${op.successorSessionId}; predecessor ${op.sourceSessionId} is audit history only. ` +
        `The host has committed worker AND task ownership and replaced the same pane. Do not adopt workers, resume or terminate the predecessor, or use any shared handoff.md. ` +
        `Use this validated handoff (also retained at ${op.artifactPath}, SHA-256 ${op.artifactHash}). Preserve pending decisions; take the stated next action within existing authority.\n${op.artifact}`,
      managerFullAccessFromConfig(),
    ),
  recoverFinishes(op) {
    for (const [id, evidence] of Object.entries(op.finishes)) {
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
