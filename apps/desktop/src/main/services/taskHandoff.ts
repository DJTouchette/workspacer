/** Host-managed task transfer. Models select task inputs once; Git commands,
 * byte custody and admission receipts stay in the workspace backends. */
import { callHub } from './hubClient';
import { pairedWorkerConnection } from './pairedWorkerConnection';

export interface HandoffSelection {
  name: string;
  kind: 'report' | 'criteria' | 'image' | 'log';
}
export interface TaskSource {
  binding: string;
  artifacts: HandoffSelection[];
  outputs: HandoffSelection[];
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

async function copyArtifacts(
  source: HandoffCall,
  destination: HandoffCall,
  binding: string,
  task: string,
  direction: 'input' | 'result',
  manifest: ArtifactManifest,
): Promise<void> {
  if (manifest.entries.length > 128) throw new Error('Task artifact count exceeds negotiated limit');
  for (const [index, entry] of manifest.entries.entries()) {
    if (!Number.isSafeInteger(entry.size) || entry.size < 0 || entry.size > 16 * 1024 * 1024)
      throw new Error('Artifact exceeds negotiated size limit');
    // Empty required files still need a durable zero-length receipt.
    for (let offset = 0; offset < entry.size || offset === 0; offset += CHUNK_BYTES) {
      const { data } = await source<{ data: string }>('agents.taskHandoff', {
        operation: 'read', binding, task, direction, index, offset,
      });
      if (Buffer.from(data, 'base64').length !== Math.min(CHUNK_BYTES, entry.size - offset))
        throw new Error('Artifact chunk length mismatch');
      await destination('agents.taskHandoff', {
        operation: 'write', binding, task, direction, index, offset, data,
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
    operation: 'freeze', binding: source.binding, task, provider, cwd,
    selections: source.artifacts, outputs: source.outputs,
  });
  await peer('agents.taskHandoff', {
    operation: 'reserve', binding: source.binding, task, plan: frozen.plan,
  });
  await local('agents.taskHandoff', { operation: 'publish', binding: source.binding, task });
  await copyArtifacts(local, peer, source.binding, task, 'input', frozen.plan.input);
  const prepared = await peer<HandoffRecord>('agents.taskHandoff', {
    operation: 'prepare', binding: source.binding, task,
  });
  if (prepared.state !== 'prepared' || prepared.digest !== frozen.digest || !prepared.allocation)
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
    operation: 'sealResult', binding, task,
  });
  if (sealed.state === 'needs-checkpoint') return sealed;
  if (sealed.state !== 'result-sealed' || !sealed.result)
    throw new Error('Output custody pending: execution host has not sealed the result');
  const receiving = await local<HandoffRecord>('agents.taskHandoff', {
    operation: 'receiveResult', binding, task, manifest: sealed.result,
  });
  if (receiving.state !== 'received')
    await copyArtifacts(peer, local, binding, task, 'result', sealed.result);
  const received = await local<HandoffRecord>('agents.taskHandoff', {
    operation: 'importResult', binding, task,
  });
  if (received.state !== 'received' || !received.custody || !received.allocation)
    throw new Error('Output custody pending: local review import incomplete');
  await peer('agents.taskHandoff', { operation: 'custody', binding, task, digest: received.custody });
  return received;
}
