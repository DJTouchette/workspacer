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
  projectCwds?: string[];
  ownerRedirects?: Record<string, string>;
  metadata: ReplacementMetadata[];
  signatures: Record<string, string>;
  finishes: Record<string, { reply: string; stopped: boolean }>;
  artifact?: string;
  artifactHash?: string;
  /** Durable ownership intent. Reconciliation rolls this forward, never back. */
  transferIntent?: boolean;
  retired?: boolean;
}
export function savedFinishDelivery(
  worker: string,
  evidence: { reply: string; stopped: boolean },
): ReplacementDelivery {
  return {
    id: `finish:${worker}`,
    kind: 'message',
    status: 'pending',
    text: `Recorded completion evidence for session:${worker}. This is not an automatic workflow verdict; inspect task history before continuing.\n${evidence.reply}`,
    error:
      'Completion was recorded, but delivery is not confirmed. Inspect the worker transcript before accounting for it or sending it again.',
  };
}

interface Journal {
  version: 1;
  operations: ReplacementRecord[];
  launches: Record<string, ManagerLaunch>;
  pendingMetadata?: Record<string, ReplacementMetadata>;
}
const MAX_BYTES = 8 * 1024 * 1024;

/** Private JSON authority for replacement lineage/parent metadata. Claudemon
 * persists stopped rows, not these fields. Never infers process liveness. */
export class ManagerReplacementState {
  private data?: Journal;
  lastError?: string;
  private active = new Map<string, number>();
  constructor(
    private filename = () => path.join(getConfigDir(), 'manager-replacements.json'),
    private projectionEnabled = true,
  ) {}
  enableProjection(): void {
    this.projectionEnabled = true;
  }
  projectedSuccessor(id: string): ReplacementRecord | undefined {
    return this.projectionEnabled
      ? this.records().find((o) => o.successorSessionId === id)
      : undefined;
  }
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
        typeof d.launches !== 'object' ||
        Array.isArray(d.launches) ||
        Object.keys(d.launches).length > 128 ||
        d.operations.length > 32 ||
        d.operations.some(
          (o) =>
            !/^[0-9a-f-]{36}$/.test(o.operationId) ||
            !o.sourceSessionId ||
            !o.successorSessionId ||
            ![
              'preparing',
              'spawning',
              'transferring',
              'binding',
              'activating',
              'complete',
              'failed',
              'cancelled',
              'recovery-required',
            ].includes(o.phase) ||
            typeof o.committed !== 'boolean' ||
            typeof o.bound !== 'boolean' ||
            typeof o.paneId !== 'string' ||
            typeof o.workspaceId !== 'string' ||
            !Array.isArray(o.workerIds) ||
            !o.workerIds.every((id) => typeof id === 'string') ||
            !Array.isArray(o.taskIds) ||
            !o.taskIds.every((id) => typeof id === 'string') ||
            !o.launch?.options?.manager ||
            o.launch.options.toolScope !== 'operator' ||
            typeof o.launch.options.cwd !== 'string' ||
            !o.signatures ||
            typeof o.signatures !== 'object' ||
            !o.finishes ||
            typeof o.finishes !== 'object' ||
            !Array.isArray(o.metadata) ||
            o.metadata.some(
              (m) => !m || typeof m.sessionId !== 'string' || typeof m.cwd !== 'string',
            ) ||
            !Array.isArray(o.deliveries) ||
            o.deliveries.some(
              (d) =>
                !d ||
                typeof d.id !== 'string' ||
                typeof d.text !== 'string' ||
                !['pending', 'sending', 'accepted', 'uncertain', 'reconciled'].includes(d.status) ||
                !['preparation', 'kickoff', 'message'].includes(d.kind),
            ),
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
        projectCwds: _p,
        ownerRedirects: _redirects,
        metadata: _m,
        signatures: _s,
        finishes: _f,
        artifact: _a,
        artifactHash: _h,
        transferIntent: _i,
        retired: _r,
        ...view
      }) =>
        structuredClone({
          ...view,
          deliveries: [
            ...view.deliveries,
            ...Object.entries(_f).map(([worker, evidence]) =>
              savedFinishDelivery(worker, evidence),
            ),
          ],
        }),
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
      this.lastError = error instanceof Error ? error.message : String(error);
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
    const launch = this.load().launches[id];
    return launch ? structuredClone(launch) : undefined;
  }
  rememberLaunch(id: string, launch: ManagerLaunch): void {
    this.edit((d) => {
      if (!d.launches[id] && Object.keys(d.launches).length >= 128)
        throw new Error('Manager launch history capacity reached');
      d.launches[id] = structuredClone(launch);
    });
  }
  rememberChild(metadata: ReplacementMetadata): void {
    if (!metadata.cwd) return;
    const parent = metadata.parentSessionId;
    const op =
      this.related(metadata.sessionId) ??
      (parent ? this.related(parent) : undefined) ??
      [...this.records()]
        .reverse()
        .find((o) => o.metadata.some((m) => m.sessionId === parent && m.isWakeTarget));
    const knownManager =
      parent &&
      (this.launch(parent)?.options.manager || this.load().pendingMetadata?.[parent]?.isWakeTarget);
    if (!op && !metadata.isWakeTarget && !knownManager) return;
    this.edit((d) => {
      if (op) {
        const record = d.operations.find((o) => o.operationId === op.operationId)!;
        const i = record.metadata.findIndex((m) => m.sessionId === metadata.sessionId);
        if (i >= 0) record.metadata[i] = structuredClone(metadata);
        else record.metadata.push(structuredClone(metadata));
      } else {
        d.pendingMetadata ??= {};
        d.pendingMetadata[metadata.sessionId] = structuredClone(metadata);
      }
    });
  }
  forgetUnclaimedMetadata(id: string): void {
    if (!this.load().pendingMetadata?.[id]) return;
    this.edit((d) => {
      delete d.pendingMetadata![id];
    });
  }
  recoveryMetadata(): ReplacementMetadata[] {
    const ids = new Set([
      ...Object.keys(this.load().pendingMetadata ?? {}),
      ...Object.keys(this.load().launches),
      ...this.records().flatMap((o) => o.metadata.map((m) => m.sessionId)),
    ]);
    return [...ids].flatMap((id) => {
      const m = this.metadata(id);
      return m ? [m] : [];
    });
  }
  updateLaunch(id: string, patch: Partial<ManagedSpawnOptions>): void {
    if (!this.launch(id)) return;
    this.edit((d) => {
      d.launches[id].options = { ...d.launches[id].options, ...patch };
    });
  }
  /** A later explicit adoption supersedes recovery routing, while the original
   * source/successor receipt remains immutable audit history. */
  noteManualReparent(
    oldId: string,
    newId: string,
    workers: string[],
    destination: ReplacementMetadata,
  ): void {
    workers = [
      ...new Set([
        ...workers,
        ...this.records().flatMap((o) =>
          o.metadata
            .filter(
              (m) => m.sessionId !== newId && this.metadata(m.sessionId)?.parentSessionId === oldId,
            )
            .map((m) => m.sessionId),
        ),
      ]),
    ];
    const affected = this.records().filter(
      (o) =>
        o.sourceSessionId === oldId ||
        o.successorSessionId === oldId ||
        o.metadata.some(
          (m) => workers.includes(m.sessionId) || (m.isWakeTarget && m.sessionId === oldId),
        ),
    );
    const unclaimed = Object.values(this.load().pendingMetadata ?? {}).filter(
      (m) => m.parentSessionId === oldId,
    );
    if (!affected.length && !unclaimed.length) return;
    this.edit((d) => {
      if (unclaimed.length) {
        d.pendingMetadata ??= {};
        for (const m of Object.values(d.pendingMetadata))
          if (m.parentSessionId === oldId && m.sessionId !== newId) m.parentSessionId = newId;
        d.pendingMetadata[newId] = structuredClone(destination);
      }
      for (const o of d.operations) if (o.ownerRedirects) delete o.ownerRedirects[newId];
      for (const o of d.operations.filter((o) =>
        affected.some((a) => a.operationId === o.operationId),
      )) {
        for (const m of o.metadata) if (workers.includes(m.sessionId)) m.parentSessionId = newId;
        if (!o.metadata.some((m) => m.sessionId === newId))
          o.metadata.push(structuredClone(destination));
        o.ownerRedirects = { ...o.ownerRedirects, [oldId]: newId };
      }
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
  parkedSuccessor(id: string): boolean {
    const op = this.related(id);
    return !!op && op.successorSessionId === id && !op.committed;
  }
  assertAvailable(id?: string): void {
    if (id && this.held(id))
      throw new Error(
        `Manager ${id} is fenced by replacement ${this.held(id)!.operationId}; inspect handoff status`,
      );
  }
  assertResume(id?: string): void {
    const op = id ? this.related(id) : undefined;
    if (!op) return;
    const primary =
      (id === op.successorSessionId && op.committed && op.phase === 'complete') ||
      (id === op.sourceSessionId &&
        !op.transferIntent &&
        ['failed', 'cancelled'].includes(op.phase));
    if (!primary)
      throw new Error(
        'This session is a retired or incomplete manager handoff. Inspect its handoff journal; stale context is not resumed.',
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
  holdMessage(id: string, text: string, signatures: Array<[string, string]> = [], sourceRequest?: ReplacementDelivery['sourceRequest']): boolean {
    const related = this.related(id);
    const o = this.held(id) ?? (related?.phase === 'activating' ? related : undefined);
    if (!o) return false;
    if (o.sourceSessionId === id && o.phase === 'complete')
      throw new Error(`Manager retired. Send to successor ${o.successorSessionId}.`);
    this.change(o.operationId, (op) => {
      if (op.deliveries.length >= 256)
        throw new Error('Handoff message capacity reached; message was not accepted');
      op.deliveries.push({ id: randomUUID(), kind: 'message', text, status: 'pending', sourceRequest });
      for (const [worker, signature] of signatures) {
        op.signatures[worker] = signature;
        delete op.finishes[worker];
      }
    });
    return true;
  }
  acknowledged(id: string): boolean {
    return this.records().some((o) =>
      o.deliveries.some((d) => d.id === id && ['accepted', 'reconciled'].includes(d.status)),
    );
  }
  noteInFlightMessage(
    parent: string,
    deliveryId: string,
    accepted: boolean,
    error?: string,
    rejected = false,
  ): void {
    const o = [...this.records()]
      .reverse()
      .find(
        (o) =>
          [o.sourceSessionId, o.successorSessionId].includes(parent) &&
          o.deliveries.some(
            (d) => d.id === deliveryId && ['sending', 'uncertain'].includes(d.status),
          ),
      );
    if (!o) return;
    this.change(o.operationId, (op) => {
      const d = op.deliveries.find((d) => d.id === deliveryId)!;
      d.status = accepted ? 'accepted' : rejected ? 'pending' : 'uncertain';
      d.error = error;
      if (!accepted) {
        op.phase = 'recovery-required';
        op.error = rejected
          ? 'An earlier message was rejected. Its text is retained for inspection.'
          : 'An in-flight message acknowledgement is uncertain. Inspect the predecessor transcript; no automatic replay.';
      }
    });
  }
  delivery(id: string, kind: ReplacementDelivery['kind'], text: string): string {
    const deliveryId = randomUUID();
    this.change(id, (o) => o.deliveries.push({ id: deliveryId, kind, text, status: 'pending' }));
    return deliveryId;
  }
  metadata(id: string): ReplacementMetadata | undefined {
    if (!this.projectionEnabled) return undefined;
    // Newest operation wins when a fleet has been handed off more than once.
    for (const o of [...this.records()].reverse()) {
      const m = o.metadata.find((m) => m.sessionId === id);
      if (!m) continue;
      return structuredClone({
        ...m,
        ...(m.parentSessionId === o.sourceSessionId && (o.committed || o.transferIntent)
          ? { parentSessionId: o.successorSessionId }
          : {}),
      });
    }
    const pending = this.load().pendingMetadata?.[id];
    if (pending) return structuredClone(pending);
    const launch = this.launch(id)?.options;
    if (launch?.manager && launch.cwd)
      return {
        sessionId: id,
        cwd: launch.cwd,
        parentSessionId: launch.parentSessionId,
        isWakeTarget: true,
        label: launch.label,
        provider: launch.provider,
        transport: launch.transport,
        settings: {
          model: launch.model,
          contextWindow: launch.contextWindow,
          effort: launch.effort,
          permissionMode: launch.permissionMode,
        },
      };
  }
  recordFinish(parent: string, worker: string, reply: string, stopped: boolean): void {
    const related = this.related(parent);
    const o = this.held(parent) ?? (related?.phase === 'activating' ? related : undefined);
    if (!o) return;
    this.change(o.operationId, (op) => {
      op.finishes[worker] = { reply, stopped };
    });
  }
  automaticWakeTarget(parent: string, seen = new Set<string>()): string {
    if (seen.has(parent)) throw new Error('Replacement ownership routing contains a cycle');
    seen.add(parent);
    const op = this.records().find((o) => o.sourceSessionId === parent && o.committed);
    return op ? this.automaticWakeTarget(op.successorSessionId, seen) : parent;
  }
  workerWakeTarget(parent: string, worker: string): string {
    return this.automaticWakeTarget(this.metadata(worker)?.parentSessionId ?? parent);
  }
  wakeTarget(parent: string, seen = new Set<string>()): string {
    if (seen.has(parent)) throw new Error('Replacement ownership routing contains a cycle');
    seen.add(parent);
    const redirect = [...this.records()].reverse().find((o) => o.ownerRedirects?.[parent])
      ?.ownerRedirects?.[parent];
    if (redirect) return this.wakeTarget(redirect, seen);
    const o = this.records().find((o) => o.sourceSessionId === parent && o.committed);
    return o ? this.wakeTarget(o.successorSessionId, seen) : parent;
  }
  signature(worker: string): string | undefined {
    for (const o of [...this.records()].reverse())
      if (o.signatures[worker]) return o.signatures[worker];
  }
  clearFinish(worker: string, expectedReply?: string): void {
    const o = [...this.records()]
      .reverse()
      .find(
        (o) =>
          o.finishes[worker] &&
          (expectedReply === undefined || o.finishes[worker].reply === expectedReply),
      );
    if (o)
      this.change(o.operationId, (op) => {
        delete op.finishes[worker];
      });
  }
  recordSignature(worker: string, signature: string): void {
    const o = [...this.records()]
      .reverse()
      .find(
        (o) =>
          o.workerIds.includes(worker) ||
          o.metadata.some((m) => m.sessionId === worker && !!m.parentSessionId),
      );
    if (o)
      this.change(o.operationId, (op) => {
        op.signatures[worker] = signature;
        delete op.finishes[worker];
      });
  }
}
export const managerReplacementState = new ManagerReplacementState(undefined, false);

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
