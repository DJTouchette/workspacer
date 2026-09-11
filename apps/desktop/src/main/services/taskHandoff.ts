/** Host-managed task transfer. Models select task inputs once; Git commands,
 * byte custody and admission receipts stay in the workspace backends. */
import { callHub } from './hubClient';
import { pairedWorkerConnection } from './pairedWorkerConnection';
import { remoteDispatchRegistry } from './remoteDispatchRegistry';
import { dispatchHistoryStore } from './dispatchHistoryStore';
import { pairedDestinationKey } from './pairedWorkerConnection';
import { createHash } from 'crypto';

export interface HandoffSelection {
  name: string;
  kind: 'report' | 'criteria' | 'image' | 'log';
}
export interface TaskSource {
  binding: string;
  artifacts: HandoffSelection[];
  outputs: HandoffSelection[];
}
export interface HandoffReceiptSelector {
  version: 1;
  allocationId: string;
  binding: string;
  digest: string;
}
interface ArtifactManifest {
  version: number;
  task: string;
  origin: string;
  producer: string;
  commit: string;
  objectFormat: string;
  entries: Array<HandoffSelection & { size: number; sha256: string }>;
}
export interface HandoffRecord {
  storage?: { usedBytes: number; reservedBytes: number; limitBytes: number; taskLimitBytes: number; retention: string };
  allocationId?: string;
  plan: {
    version: number;
    binding: string;
    revision: string;
    provider: string;
    input: ArtifactManifest;
    outputs: HandoffSelection[];
  };
  digest: string;
  state: string;
  allocation?: string;
  result?: ArtifactManifest;
  custody?: string;
}
export type HandoffCall = <T>(method: string, params: unknown) => Promise<T>;
const remote: HandoffCall = (method, params) => pairedWorkerConnection.call(method, params);
const CHUNK_BYTES = 256 * 1024;

export async function prepareLocalTaskHandoff(
  source: TaskSource,
  taskId: string | undefined,
  predecessor: string | undefined,
  ownerSessionId: string | undefined,
  provider: string,
  cwd: string,
): Promise<HandoffRecord> {
  const task = taskId ? dispatchHistoryStore.task(taskId) : undefined;
  const attempt = task?.attempts.find((a) => a.dispatchId === predecessor);
  const record = remoteDispatchRegistry.list().find((r) => r.localSessionId === attempt?.sessionId);
  if (
    !task ||
    task.ownerSessionId !== ownerSessionId ||
    !predecessor ||
    !record?.handoff ||
    record.handoff.binding !== source.binding ||
    record.handoff.state !== 'received' ||
    record.peer !== pairedDestinationKey()
  )
    throw new Error(
      'Local continuation requires the preceding verified handoff in this same owned task',
    );
  // One admission per exact task predecessor. A lost launch reply cannot
  // allocate another worker by generating another transfer nonce.
  const nextTask = createHash('sha256')
    .update(JSON.stringify([taskId, predecessor, source.binding]))
    .digest('hex');
  await callHub('agents.taskHandoff', {
    operation: 'freeze',
    binding: source.binding,
    task: nextTask,
    fromTask: record.dispatchId,
    provider,
    cwd,
    selections: source.artifacts,
    outputs: source.outputs,
  });
  const prepared = await callHub<HandoffRecord>('agents.taskHandoff', {
    operation: 'prepareLocal',
    binding: source.binding,
    task: nextTask,
  });
  if (prepared.state !== 'prepared' || !prepared.allocation)
    throw new Error('Local handoff checkpoint or required artifacts are not verified');
  await callHub('agents.taskHandoff', {
    operation: 'claimLocal',
    binding: source.binding,
    task: nextTask,
    digest: prepared.digest,
  });
  return prepared;
}

/** Called only by the local host-user Task Inspector action, never by a
 * worker's reported test/pass claim or terminal-message acknowledgment. */
export async function setTaskHandoffDisposition(
  request: Extract<
    import('../shared/dispatchHistory').TaskEditRequest,
    { action: 'handoff-disposition' }
  >,
): Promise<import('../shared/dispatchHistory').TaskEditResponse> {
  try {
    const task = dispatchHistoryStore.task(request.taskId);
    const attempt = task?.attempts.find((a) => a.dispatchId === request.dispatchId);
    if (!task || (task.revision ?? 0) !== request.expectedTaskRevision)
      return {
        ok: false,
        code: 'conflict',
        error: 'Task changed; refresh before accepting outputs',
        task,
      };
    const record = remoteDispatchRegistry
      .list()
      .find((r) => r.localSessionId === attempt?.sessionId);
    if (
      !attempt?.handoff ||
      attempt.handoff.state !== 'received' ||
      !record?.handoff ||
      record.peer !== pairedDestinationKey()
    )
      throw new Error('Verified task custody is unavailable for this pairing');
    const p = { binding: record.handoff.binding, task: record.dispatchId };
    const receipt = await callHub<HandoffRecord>('agents.taskHandoff', {
      ...p,
      operation: 'status',
    });
    if (!receipt.custody) throw new Error('Durable output custody is missing');
    const disposition = {
      ...p,
      operation: 'disposition',
      digest: receipt.custody,
      keep: request.keep,
    };
    const target = await remote<HandoffRecord>('agents.taskHandoff', disposition);
    await callHub('agents.taskHandoff', disposition);
    dispatchHistoryStore.observeHandoff(attempt.sessionId, {
      ...attempt.handoff,
      disposition: request.keep ? 'keep' : 'accepted',
      note: target.state === 'cleaned'
        ? 'The execution copy has been cleaned. Local review custody is retained.'
        : receipt.storage?.retention,
    });
    return { ok: true, task: dispatchHistoryStore.task(request.taskId)! };
  } catch (error) {
    return { ok: false, code: 'unavailable', error: String(error) };
  }
}

async function copyArtifacts(
  source: HandoffCall,
  destination: HandoffCall,
  binding: string,
  task: string,
  direction: 'input' | 'result',
  manifest: ArtifactManifest,
): Promise<void> {
  if (manifest.entries.length > 128)
    throw new Error('Task artifact count exceeds negotiated limit');
  for (const [index, entry] of manifest.entries.entries()) {
    if (!Number.isSafeInteger(entry.size) || entry.size < 0 || entry.size > 16 * 1024 * 1024)
      throw new Error('Artifact exceeds negotiated size limit');
    // Empty required files still need a durable zero-length receipt.
    for (let offset = 0; offset < entry.size || offset === 0; offset += CHUNK_BYTES) {
      const { data } = await source<{ data: string }>('agents.taskHandoff', {
        operation: 'read',
        binding,
        task,
        direction,
        index,
        offset,
      });
      if (Buffer.from(data, 'base64').length !== Math.min(CHUNK_BYTES, entry.size - offset))
        throw new Error('Artifact chunk length mismatch');
      await destination('agents.taskHandoff', {
        operation: 'write',
        binding,
        task,
        direction,
        index,
        offset,
        data,
      });
    }
  }
}

export async function prepareTaskHandoff(
  task: string,
  source: TaskSource,
  provider: string,
  cwd: string,
  local: HandoffCall = callHub,
  peer: HandoffCall = remote,
): Promise<HandoffRecord> {
  const frozen = await local<HandoffRecord>('agents.taskHandoff', {
    operation: 'freeze',
    binding: source.binding,
    task,
    provider,
    cwd,
    selections: source.artifacts,
    outputs: source.outputs,
  });
  await peer('agents.taskHandoff', {
    operation: 'reserve',
    binding: source.binding,
    task,
    plan: frozen.plan,
  });
  await local('agents.taskHandoff', { operation: 'publish', binding: source.binding, task });
  await copyArtifacts(local, peer, source.binding, task, 'input', frozen.plan.input);
  const prepared = await peer<HandoffRecord>('agents.taskHandoff', {
    operation: 'prepare',
    binding: source.binding,
    task,
  });
  if (prepared.state !== 'prepared' || prepared.digest !== frozen.digest || !prepared.allocation || !prepared.allocationId)
    throw new Error('Target did not verify the selected checkpoint and required artifacts');
  return prepared;
}

export async function importTaskHandoffResult(
  task: string,
  binding: string,
  local: HandoffCall = callHub,
  peer: HandoffCall = remote,
): Promise<HandoffRecord> {
  const sealed = await peer<HandoffRecord>('agents.taskHandoff', {
    operation: 'sealResult',
    binding,
    task,
  });
  if (sealed.state === 'needs-checkpoint') return sealed;
  if (sealed.state !== 'result-sealed' || !sealed.result)
    throw new Error('Output custody pending: execution host has not sealed the result');
  const receiving = await local<HandoffRecord>('agents.taskHandoff', {
    operation: 'receiveResult',
    binding,
    task,
    manifest: sealed.result,
  });
  if (receiving.state !== 'received')
    await copyArtifacts(peer, local, binding, task, 'result', sealed.result);
  const received = await local<HandoffRecord>('agents.taskHandoff', {
    operation: 'importResult',
    binding,
    task,
  });
  if (received.state !== 'received' || !received.custody || !received.allocation)
    throw new Error('Output custody pending: local review import incomplete');
  await peer('agents.taskHandoff', {
    operation: 'custody',
    binding,
    task,
    digest: received.custody,
  });
  return received;
}
