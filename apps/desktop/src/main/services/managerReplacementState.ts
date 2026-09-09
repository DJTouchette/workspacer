import fs from 'fs';
import path from 'path';
import { randomUUID } from 'crypto';
import { getConfigDir } from './configService';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import type { ManagedSpawnOptions } from './managedSpawn';
import type { SessionSpawnSettings, SessionRouting } from './claudeSessionStore';
import type { ManagerReplacementView, ReplacementDelivery } from '../shared/managerReplacement';

export interface ReplacementMetadata {
  sessionId: string;
  cwd: string;
  label?: string;
  parentSessionId?: string;
  isWakeTarget?: boolean;
  provider?: string;
  transport?: 'pty' | 'stream';
  settings?: SessionSpawnSettings;
  resultSchema?: Record<string, unknown>;
  routing?: SessionRouting;
}
export interface ManagerLaunch {
  options: ManagedSpawnOptions;
  grants: string;
  configuration?: string;
}
export interface ReplacementRecord extends ManagerReplacementView {
  launch: ManagerLaunch;
  metadata: ReplacementMetadata[];
  signatures: Record<string, string>;
  finishes: Record<string, { reply: string; stopped: boolean }>;
  artifact?: string;
  artifactHash?: string;
  /** Durable ownership intent. Reconciliation rolls this forward, never back. */
  transferIntent?: boolean;
  retired?: boolean;
}
interface Journal {
  version: 1;
  operations: ReplacementRecord[];
  launches: Record<string, ManagerLaunch>;
}
const MAX_BYTES = 8 * 1024 * 1024;

/** Private JSON authority for replacement lineage/parent metadata. Claudemon
 * persists stopped rows, not these fields. Never infers process liveness. */
export class ManagerReplacementState {
  private data?: Journal;
  private active = new Map<string, number>();
  constructor(private filename = () => path.join(getConfigDir(), 'manager-replacements.json')) {}
  private load(): Journal {
    if (this.data) return this.data;
    try {
      if (fs.statSync(this.filename()).size > MAX_BYTES)
        throw new Error('Replacement journal is oversized');
      const d = JSON.parse(fs.readFileSync(this.filename(), 'utf8')) as Journal;
      if (
        d.version !== 1 ||
        !Array.isArray(d.operations) ||
        !d.launches ||
        d.operations.length > 32 ||
        d.operations.some(
          (o) =>
            !o.operationId ||
            !o.sourceSessionId ||
            !o.successorSessionId ||
            !Array.isArray(o.metadata) ||
            !Array.isArray(o.deliveries),
        )
      )
        throw new Error(
          'Replacement journal is invalid; inspect the saved journal before continuing',
        );
      this.data = d;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      this.data = { version: 1, operations: [], launches: {} };
    }
    return this.data;
  }
  records(): ReplacementRecord[] {
    return this.load().operations;
  }
  views(): ManagerReplacementView[] {
    return this.records().map(
      ({
        launch: _l,
        metadata: _m,
        signatures: _s,
        finishes: _f,
        artifact: _a,
        artifactHash: _h,
        transferIntent: _i,
        retired: _r,
        ...view
      }) => structuredClone(view),
    );
  }
  edit(fn: (d: Journal) => void): void {
    const before = structuredClone(this.load());
    try {
      fn(this.data!);
      const json = JSON.stringify(this.data);
      if (Buffer.byteLength(json) > MAX_BYTES)
        throw new Error(
          'Replacement journal capacity reached; retained evidence was not discarded',
        );
      atomicWriteFileSync(this.filename(), json, { mode: 0o600 });
    } catch (error) {
      this.data = before;
      throw error;
    }
  }
  change(id: string, fn: (o: ReplacementRecord) => void): void {
    this.edit((d) => {
      const op = d.operations.find((o) => o.operationId === id);
      if (!op) throw new Error('Unknown manager replacement');
      fn(op);
      op.updatedAt = Date.now();
    });
  }
  get(id: string): ReplacementRecord {
    const o = this.records().find((o) => o.operationId === id);
    if (!o) throw new Error('Unknown manager replacement');
    return o;
  }
  launch(id: string): ManagerLaunch | undefined {
    return this.load().launches[id];
  }
  rememberLaunch(id: string, launch: ManagerLaunch): void {
    this.edit((d) => {
      if (!d.launches[id] && Object.keys(d.launches).length >= 128)
        throw new Error('Manager launch history capacity reached');
      d.launches[id] = structuredClone(launch);
    });
  }
  related(id: string): ReplacementRecord | undefined {
    return [...this.records()]
      .reverse()
      .find((o) => o.sourceSessionId === id || o.successorSessionId === id);
  }
  held(id: string): ReplacementRecord | undefined {
    const o = this.related(id);
    if (!o) return;
    if (o.sourceSessionId === id && (o.committed || o.transferIntent)) return o;
    if (o.successorSessionId === id && o.phase === 'activating' && o.committed) return;
    if (!['complete', 'failed', 'cancelled'].includes(o.phase)) return o;
  }
  assertAvailable(id?: string): void {
    if (id && this.held(id))
      throw new Error(
        `Manager ${id} is fenced by replacement ${this.held(id)!.operationId}; inspect handoff status`,
      );
  }
  assertResume(id?: string): void {
    if (id && this.related(id))
      throw new Error(
        'This manager has a handoff journal. Inspect handoff status; automatic resume of its old context is unavailable.',
      );
  }
  async admitted<T>(ids: (string | undefined)[], action: () => Promise<T>): Promise<T> {
    const keys = [...new Set(ids.filter((s): s is string => !!s))];
    keys.forEach((id) => this.assertAvailable(id));
    keys.forEach((id) => this.active.set(id, (this.active.get(id) ?? 0) + 1));
    try {
      return await action();
    } finally {
      keys.forEach((id) => this.active.set(id, (this.active.get(id) ?? 1) - 1));
    }
  }
  activeCount(id: string): number {
    return this.active.get(id) ?? 0;
  }
  holdMessage(id: string, text: string, signatures: Array<[string, string]> = []): boolean {
    const o = this.held(id);
    if (!o) return false;
    if (o.sourceSessionId === id && o.phase === 'complete')
      throw new Error(`Manager retired. Send to successor ${o.successorSessionId}.`);
    this.change(o.operationId, (op) => {
      if (op.deliveries.length >= 256)
        throw new Error('Handoff message capacity reached; message was not accepted');
      op.deliveries.push({ id: randomUUID(), kind: 'message', text, status: 'pending' });
      for (const [worker, signature] of signatures) {
        op.signatures[worker] = signature;
        delete op.finishes[worker];
      }
    });
    return true;
  }
  delivery(id: string, kind: ReplacementDelivery['kind'], text: string): string {
    const deliveryId = randomUUID();
    this.change(id, (o) => o.deliveries.push({ id: deliveryId, kind, text, status: 'pending' }));
    return deliveryId;
  }
  metadata(id: string): ReplacementMetadata | undefined {
    // Newest operation wins when a fleet has been handed off more than once.
    for (const o of [...this.records()].reverse()) {
      const m = o.metadata.find((m) => m.sessionId === id);
      if (!m) continue;
      return {
        ...m,
        ...(m.parentSessionId === o.sourceSessionId && (o.committed || o.transferIntent)
          ? { parentSessionId: o.successorSessionId }
          : {}),
      };
    }
  }
  recordFinish(parent: string, worker: string, reply: string, stopped: boolean): void {
    const o = this.held(parent);
    if (!o) return;
    this.change(o.operationId, (op) => {
      op.finishes[worker] = { reply, stopped };
    });
  }
  wakeTarget(parent: string): string {
    const o = this.records().find((o) => o.sourceSessionId === parent && o.committed);
    return o ? this.wakeTarget(o.successorSessionId) : parent;
  }
  signature(worker: string): string | undefined {
    for (const o of [...this.records()].reverse())
      if (o.signatures[worker]) return o.signatures[worker];
  }
  recordSignature(worker: string, signature: string): void {
    const o = [...this.records()].reverse().find((o) => o.workerIds.includes(worker));
    if (o)
      this.change(o.operationId, (op) => {
        op.signatures[worker] = signature;
        delete op.finishes[worker];
      });
  }
}
export const managerReplacementState = new ManagerReplacementState();

export function managerDispatch<T>(
  fn: (params: unknown) => Promise<T>,
): (params: unknown) => Promise<T> {
  return (raw) => {
    const p = (raw ?? {}) as {
      dispatchOwnerSessionId?: string;
      parentSessionId?: string;
      resumeSessionId?: string;
    };
    managerReplacementState.assertResume(p.resumeSessionId);
    return managerReplacementState.admitted([p.dispatchOwnerSessionId, p.parentSessionId], () =>
      fn(raw),
    );
  };
}
