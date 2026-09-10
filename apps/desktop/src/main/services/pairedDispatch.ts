import { randomBytes, randomUUID } from 'crypto';
import path from 'path';
import { getConfigDir } from './configService';
import { pairedWorkerConnection as connection, pairedDestinationKey } from './pairedWorkerConnection';
import { getPairedWorkerTarget } from './remoteServer';
import { remoteDispatchRegistry as registry, sanitizeRemoteEntry, DISPATCH_PROTOCOL } from './remoteDispatchRegistry';
import { claudeSessionStore } from './claudeSessionStore';
import { managerReplacementState } from './managerReplacementState';
import { dispatchHistoryStore } from './dispatchHistoryStore';
import { readStructuredResult, buildResultContract } from '../shared/structuredResult';
import { readWorkerEscalation } from '../shared/workerEscalation';
import { buildFleetMessage } from '../shared/fleetMessages';
import { workflowWakeInstructions } from './fleetWorkflowRuntime';
import { claudemonSessionClient } from './claudemonSessionClient';
import { renderDispatchTemplate } from '../lib/dispatchTemplate';

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
  connection.onDisconnected = () => claudeSessionStore.markHubPeerOffline('paired');
  connection.onConnected = () => {
    for (const record of registry.openForPeer(pairedDestinationKey())) {
      void connection.call('agents.dispatchReplay', { dispatchId: record.dispatchId }).catch(() => {});
      if (record.sessionId && record.localSessionId) void connection.call<import('./claudeSessionStore').RemoteSnapshotWire>('sessions.snapshot', {sessionId:record.sessionId}).then((snapshot) => claudeSessionStore.upsertRemoteSession('paired', {...snapshot,sessionId:record.localSessionId,parentSessionId:record.ownerSessionId,isWakeTarget:false})).catch(() => {});
    }
  };
  connection.onEvent = (event) => {
    if (event.type === 'agent.snapshot') {
      const snapshot = event.data as import('./claudeSessionStore').RemoteSnapshotWire;
      const record = registry.list().find((r) => r.peer === pairedDestinationKey() && r.sessionId === snapshot?.sessionId);
      if (record?.localSessionId) claudeSessionStore.upsertRemoteSession('paired', { ...snapshot, sessionId:record.localSessionId, parentSessionId:record.ownerSessionId, isWakeTarget:false });
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
  const entry = sanitizeRemoteEntry(update.entry);
  entry.sessionId = record.localSessionId;
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
    dispatchHistoryStore.validated(entry.sessionId, entry.escalation ? 'escalated' : entry.result ? 'valid' : entry.resultError ? 'invalid' : 'absent', undefined, entry.result ? JSON.parse(entry.result) : entry.escalation);
  }
  dispatchHistoryStore.observe({ sessionId: entry.sessionId, status: 'active', ambientState: update.final ? 'idle' : 'streaming', pendingApproval: null, pendingQuestions: null, usage: null, statusLine: undefined, hub: undefined });
  const text = buildFleetMessage(update.kind, [entry]) + workflowWakeInstructions([entry.sessionId]);
  const signatures: Array<[string,string]> = [[entry.sessionId, `paired:${record.dispatchId}:${update.seq}`]];
  if (managerReplacementState.signature(entry.sessionId) !== signatures[0][1]) {
    registry.beginDelivery(record.dispatchId, update.seq);
    const response = await claudemonSessionClient.message(parentSessionId, text, signatures);
    if (!response.ok) throw new Error('Remote result wake was not accepted');
    managerReplacementState.recordSignature(entry.sessionId, signatures[0][1]);
  }
  registry.acknowledge(record.dispatchId, update);
}

type Admission = Parameters<typeof dispatchHistoryStore.accept>[0];

export async function spawnPairedWorker(
  p: Record<string, unknown>,
  admission: Omit<Admission, 'sessionId' | 'executionCwd'>,
  templateBody?: string,
  resultSchema?: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  startPairedDispatch();
  if (p.executionTarget !== 'paired' || !getPairedWorkerTarget()) throw new Error('Paired worker target is not enabled');
  const owner = admission.owner;
  if (!owner?.isWakeTarget || owner.hub || owner.status === 'ended' || p.parentSessionId !== owner.sessionId) throw new Error('Paired dispatch requires the authenticated live local manager');
  if (p.manager || p.resumeSessionId || p.retrySourceSessionId || p.profileId || p.mcpItemIds || p.pluginTools || p.launchIntegrationId) throw new Error('Paired workers require a fresh session without local profile or process configuration');
  const capabilities = await connection.call<{ protocol: number; executes: boolean; cwds: Array<{path:string}>; providers: Array<{provider:string;found:boolean;authenticated:boolean|null}> }>('fleet.dispatchCapabilities');
  if (capabilities.protocol !== DISPATCH_PROTOCOL || !capabilities.executes) throw new Error('Paired worker dispatch unsupported; upgrade the older endpoint');
  if (!capabilities.cwds?.some((c) => c.path === p.remoteCwd)) throw new Error('Choose an actual remote cwd from list_dispatch_targets');
  if (!capabilities.providers?.some((provider) => provider.provider === p.provider && provider.found && provider.authenticated === true)) throw new Error('Selected provider is not authenticated on the remote host');
  const dispatchId = randomBytes(32).toString('hex');
  const localSessionId = `paired:${randomUUID()}`;
  const record = registry.open({ dispatchId, peer: pairedDestinationKey(), ownerSessionId: owner.sessionId, localSessionId, resultSchema, cwd: p.remoteCwd, provider: p.provider, model: p.model, label: p.label });
  if (!record) throw new Error('Could not persist paired dispatch admission');
  const remoteOrigin = { protocol: DISPATCH_PROTOCOL, dispatchId };
  const prepared = await connection.call<{cwd:string;repo:string;worktree:boolean;branch?:string}>('agents.dispatchPrepare', {remoteOrigin, cwd:p.remoteCwd, provider:p.provider, worktree:p.worktree === true});
  const worktree = { requested:p.worktree === true, allocated:prepared.worktree, fallback:false, branch:prepared.branch };
  if (p.worktree && !prepared.worktree) throw new Error('Remote isolated worktree required');
  // Book the local workflow attempt before starting the remote process. A lost
  // reply leaves this attempt pending and prevents blindly repeating the step.
  const ids = dispatchHistoryStore.accept({ ...admission, sessionId:localSessionId, executionCwd:prepared.cwd, provider:String(p.provider), requestedProvider:String(p.provider), requestedModel:typeof p.model === 'string' ? p.model : undefined, worktree, executionTarget:'paired' });
  let message = templateBody ? renderDispatchTemplate(templateBody, (p.templateParams ?? {}) as Record<string,string>, {cwd:prepared.cwd, projectCwd:prepared.repo}) : p.message;
  if (resultSchema) message = `${message ?? ''}\n\n${buildResultContract(resultSchema)}`;
  // Allowlist supported execution metadata. All origin workflow/request and
  // grant fields stay local. The remote token and remote routing ceiling win.
  const wire: Record<string,unknown> = { remoteOrigin, cwd:prepared.cwd, provider:p.provider, transport:'stream', message, skipPermissions:false, toolScope:p.toolScope, label:p.label, model:p.model, effort:p.effort, role:p.role, capability:p.capability, decisionId:p.decisionId };
  try {
    const result = await connection.call<{sessionId?:string;messageQueued?:boolean}>('agents.spawn',wire);
    if (!result.sessionId || result.messageQueued !== true) throw new Error('Remote admission or initial delivery is uncertain; do not repeat spawn or send the initial message');
    registry.attachSession(dispatchId,result.sessionId);
    const snapshot = await connection.call<import('./claudeSessionStore').RemoteSnapshotWire>('sessions.snapshot', {sessionId:result.sessionId}).catch(() => ({cwd:prepared.cwd,status:'starting' as const}));
    claudeSessionStore.upsertRemoteSession('paired', {...snapshot,sessionId:localSessionId,parentSessionId:owner.sessionId,isWakeTarget:false});
    return {...ids,sessionId:localSessionId,remoteSessionId:result.sessionId,executionTarget:'paired',executionCwd:prepared.cwd,worktreeResult:worktree,messageQueued:true};
  } catch (err) {
    registry.fail(dispatchId,'Remote admission unknown; reconcile this dispatch, never blindly repeat it');
    throw err;
  }
}

export async function selectPairedModel(raw: unknown): Promise<unknown> {
  const p = (raw ?? {}) as Record<string,unknown>;
  if (!getPairedWorkerTarget()) throw new Error('Paired worker target is not enabled');
  const caps = await connection.call<{protocol:number;executes:boolean;cwds:Array<{path:string}>;providers:Array<{provider:string;authenticated:boolean|null;found:boolean}>}>('fleet.dispatchCapabilities');
  if (caps.protocol !== DISPATCH_PROTOCOL || !caps.executes) throw new Error('Paired dispatch unsupported; upgrade the older endpoint');
  if (!caps.cwds.some((c) => c.path === p.cwd)) throw new Error('Select a remote repository returned by list_dispatch_targets');
  if (!caps.providers.some((v) => v.provider === p.provider && v.found && v.authenticated === true)) throw new Error('Explicitly choose a provider authenticated on the remote host');
  const {role,provider,cwd,difficulty,risk,decisionDensity} = p;
  return connection.call('routing.select',{role,provider,cwd,difficulty,risk,decisionDensity});
}
