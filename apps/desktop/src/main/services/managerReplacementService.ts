import { randomUUID } from 'crypto';
import {
  ManagerReplacementState,
  type ManagerLaunch,
  type ReplacementMetadata,
  type ReplacementRecord,
} from './managerReplacementState';
import {
  createArtifactPath,
  preparationPrompt,
  validateManagerArtifact,
} from './managerReplacementArtifact';
import type {
  ManagerReplacementRequest,
  ManagerReplacementResponse,
} from '../shared/managerReplacement';

export interface ReplacementHost {
  source(id: string, paneId?: string): ManagerLaunch;
  inventory(id: string): ReplacementMetadata[];
  tasks(id: string): string[];
  signatures(ids: string[]): Record<string, string>;
  finishes(id: string): Record<string, { reply: string; stopped: boolean }>;
  receipt(id: string): string;
  settled(id: string): boolean;
  spawn(id: string, launch: ManagerLaunch): Promise<void>;
  validateSuccessor(id: string, launch: ManagerLaunch): Promise<void>;
  transfer(source: string, successor: string, operationId: string): void;
  restore(metadata: ReplacementMetadata[]): Promise<void>;
  bound(paneId: string, id: string): boolean;
  send(id: string, text: string): Promise<{ ok: boolean; mode?: string }>;
  pause(id: string): Promise<void>;
  close(id: string): Promise<void>;
  kickoff(op: ReplacementRecord): string;
  recoverFinishes(op: ReplacementRecord): void;
}
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const same = (a: string[], b: string[]) =>
  JSON.stringify([...a].sort()) === JSON.stringify([...b].sort());
const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Main-process transaction, never renderer orchestration. All uncertain sends
 * retain their text and require explicit reconciliation; this is NOT exactly once. */
export class ManagerReplacementService {
  private running = new Map<string, Promise<void>>();
  private initialized?: Promise<void>;
  constructor(
    readonly state: ManagerReplacementState,
    private host: ReplacementHost,
    private timing = { preparationMs: 180_000, pollMs: 250, deliveryMs: 15_000 },
  ) {}
  initialize(): Promise<void> {
    return (this.initialized ??= this.recover());
  }
  private async recover(): Promise<void> {
    for (const saved of this.state.records()) {
      const id = saved.operationId;
      this.state.change(id, (o) => {
        for (const d of o.deliveries)
          if (d.status === 'sending') {
            d.status = 'uncertain';
            d.error = 'Desktop stopped before daemon acknowledgement was recorded';
          }
        if (!['complete', 'failed', 'cancelled'].includes(o.phase)) {
          o.phase = 'recovery-required';
          o.error =
            'Desktop restarted during handoff. Inspect retained delivery evidence; nothing was replayed.';
        }
      });
      const o = this.state.get(id);
      // Restore metadata without inventing liveness, before releasing any gate.
      await this.host.restore(
        o.metadata.map((m) => ({
          ...m,
          ...(m.parentSessionId === o.sourceSessionId && (o.committed || o.transferIntent)
            ? { parentSessionId: o.successorSessionId }
            : {}),
        })),
      );
      if (o.transferIntent && o.phase !== 'complete') await this.reconcile(id);
      else if (o.phase === 'recovery-required') {
        await this.host.pause(o.successorSessionId).catch(() => {});
      }
      this.host.recoverFinishes(this.state.get(id));
    }
  }
  async request(request: ManagerReplacementRequest): Promise<ManagerReplacementResponse> {
    try {
      await this.initialize();
      if (request.action === 'start') this.start(request.sourceSessionId, request.paneId);
      else if (request.action === 'cancel') await this.cancel(request.operationId);
      else if (request.action === 'reconcile') await this.reconcile(request.operationId);
      else if (request.action === 'bind') await this.bind(request.operationId);
      else if (request.action === 'resolve-delivery') {
        if (request.acknowledgeDuplicateRisk !== true)
          throw new Error('Explicit acknowledgement is required: retry may duplicate agent work');
        const op = this.state.get(request.operationId);
        const delivery = op.deliveries.find((d) => d.id === request.deliveryId);
        if (!delivery || !['uncertain', 'pending'].includes(delivery.status))
          throw new Error('Delivery is not awaiting resolution');
        this.state.change(op.operationId, (o) => {
          const d = o.deliveries.find((d) => d.id === request.deliveryId)!;
          d.status = request.resolution === 'accepted' ? 'reconciled' : 'pending';
          d.error =
            request.resolution === 'accepted'
              ? 'User inspected the destination and confirmed delivery'
              : 'User explicitly requested retry; it may duplicate work';
        });
        if (request.resolution === 'retry')
          await this.deliver(
            op.operationId,
            delivery.id,
            op.committed ? op.successorSessionId : op.sourceSessionId,
          );
        await this.reconcile(op.operationId);
      }
      return { available: true, operations: this.state.views() };
    } catch (error) {
      return { available: true, operations: this.safeViews(), error: errorText(error) };
    }
  }
  private safeViews() {
    try {
      return this.state.views();
    } catch {
      return [];
    }
  }
  start(sourceSessionId: string, paneId: string): string {
    if (!sourceSessionId || !paneId || sourceSessionId.length > 128 || paneId.length > 256)
      throw new Error('Source session and owning pane are required');
    const previous = this.state
      .records()
      .find(
        (o) =>
          o.sourceSessionId === sourceSessionId &&
          (!['failed', 'cancelled'].includes(o.phase) ||
            o.deliveries.some((d) => d.status === 'uncertain' || d.status === 'pending')),
      );
    if (previous) return previous.operationId;
    const launch = this.host.source(sourceSessionId, paneId);
    const metadata = this.host.inventory(sourceSessionId);
    const workerIds = metadata
      .filter((m) => m.parentSessionId === sourceSessionId)
      .map((m) => m.sessionId);
    const taskIds = this.host.tasks(sourceSessionId);
    const operationId = randomUUID();
    const artifactPath = createArtifactPath(launch.options.cwd!, operationId);
    const now = Date.now();
    this.state.edit((d) => {
      if (d.operations.length >= 32)
        throw new Error(
          'Replacement journal capacity reached; retained audit history was not removed',
        );
      d.operations.push({
        operationId,
        sourceSessionId,
        successorSessionId: randomUUID(),
        paneId,
        phase: 'preparing',
        createdAt: now,
        updatedAt: now,
        committed: false,
        bound: false,
        artifactPath,
        workerIds,
        taskIds,
        deliveries: [],
        launch,
        metadata,
        signatures: this.host.signatures(workerIds),
        finishes: this.host.finishes(sourceSessionId),
      });
    });
    this.run(operationId, () => this.prepare(operationId));
    return operationId;
  }
  private run(id: string, action: () => Promise<void>): void {
    if (this.running.has(id)) return;
    const run = action()
      .catch((error) => this.fail(id, error))
      .finally(() => this.running.delete(id));
    this.running.set(id, run);
  }
  async idle(id: string): Promise<void> {
    await this.running.get(id);
  }
  private async prepare(id: string): Promise<void> {
    const deadline = Date.now() + this.timing.preparationMs;
    while (this.state.activeCount(this.state.get(id).sourceSessionId)) {
      if (Date.now() >= deadline)
        throw new Error('Pending dispatch did not settle before handoff timeout');
      await wait(this.timing.pollMs);
    }
    let o = this.state.get(id);
    if (o.phase !== 'preparing') return;
    // Include dispatches accepted before the fence, even if their registration
    // or task-history acceptance completed after the click.
    const metadata = this.host.inventory(o.sourceSessionId);
    this.state.change(id, (op) => {
      op.metadata = metadata;
      op.workerIds = metadata
        .filter((m) => m.parentSessionId === op.sourceSessionId)
        .map((m) => m.sessionId);
      op.taskIds = this.host.tasks(op.sourceSessionId);
    });
    o = this.state.get(id);
    const preparation = this.state.delivery(id, 'preparation', preparationPrompt(o));
    if (!(await this.deliver(id, preparation, o.sourceSessionId))) return;
    let artifact: { raw: string; hash: string } | undefined;
    while (Date.now() < deadline) {
      o = this.state.get(id);
      if (o.phase !== 'preparing') return;
      const receipt = this.host.receipt(o.sourceSessionId);
      if (receipt.includes('```wks-manager-handoff') && receipt.includes(id)) {
        artifact = validateManagerArtifact(o, receipt); // invalid explicit receipt fails closed
        if (this.host.settled(o.sourceSessionId)) break;
      }
      await wait(this.timing.pollMs);
    }
    if (!artifact || !this.host.settled(o.sourceSessionId))
      throw new Error(
        'Checkpoint/handoff timed out. The old manager remains available; inspect its reply and retry.',
      );
    const current = this.host.source(o.sourceSessionId, o.paneId);
    if (JSON.stringify(current) !== JSON.stringify(o.launch))
      throw new Error('Manager settings, identity or grants changed during preparation');
    const currentWorkers = this.host
      .inventory(o.sourceSessionId)
      .filter((m) => m.parentSessionId === o.sourceSessionId)
      .map((m) => m.sessionId);
    if (!same(currentWorkers, o.workerIds) || !same(this.host.tasks(o.sourceSessionId), o.taskIds))
      throw new Error(
        'Owned workers or tasks changed during preparation; retry with a fresh checkpoint',
      );
    this.state.change(id, (op) => {
      op.artifact = artifact!.raw;
      op.artifactHash = artifact!.hash;
      op.phase = 'spawning';
    });
    await this.host.spawn(o.successorSessionId, o.launch);
    if (this.state.get(id).phase !== 'spawning') {
      await this.host.close(o.successorSessionId);
      return;
    }
    await this.host.validateSuccessor(o.successorSessionId, o.launch);
    this.state.change(id, (op) => {
      op.metadata.push({
        sessionId: op.successorSessionId,
        cwd: op.launch.options.cwd!,
        provider: op.launch.options.provider,
        isWakeTarget: true,
        label: 'Fleet Manager',
        transport: 'stream',
        settings: {
          model: op.launch.options.model,
          contextWindow: op.launch.options.contextWindow,
          effort: op.launch.options.effort,
          permissionMode: op.launch.options.permissionMode,
        },
      });
      op.phase = 'transferring';
      op.transferIntent = true;
    });
    this.host.transfer(o.sourceSessionId, o.successorSessionId, id);
    this.state.change(id, (op) => {
      op.committed = true;
      op.phase = 'binding';
    });
  }
  private async deliver(id: string, deliveryId: string, target: string): Promise<boolean> {
    const delivery = this.state.get(id).deliveries.find((d) => d.id === deliveryId)!;
    if (['accepted', 'reconciled'].includes(delivery.status)) return true;
    if (delivery.status !== 'pending') return false;
    this.state.change(id, (o) => {
      o.deliveries.find((d) => d.id === deliveryId)!.status = 'sending';
    });
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const result = await Promise.race([
        this.host.send(target, delivery.text),
        new Promise<never>((_, reject) => {
          timer = setTimeout(
            () => reject(new Error('Timed out awaiting daemon acknowledgement')),
            this.timing.deliveryMs,
          );
        }),
      ]);
      if (!result.ok) throw new Error(`Daemon refused delivery (${result.mode ?? 'unavailable'})`);
      this.state.change(id, (o) => {
        o.deliveries.find((d) => d.id === deliveryId)!.status = 'accepted';
      });
      return true;
    } catch (error) {
      this.state.change(id, (o) => {
        const d = o.deliveries.find((d) => d.id === deliveryId)!;
        d.status = 'uncertain';
        d.error = errorText(error);
        o.phase = 'recovery-required';
        o.error =
          'Delivery acknowledgement is uncertain. Inspect the destination before explicitly retrying; retry can duplicate work.';
      });
      // Best effort only: providers may already have acted. The host dispatch
      // fence remains authoritative even if interruption fails.
      await this.host.pause(target).catch(() => {});
      return false;
    } finally {
      if (timer) clearTimeout(timer);
    }
  }
  private async fail(id: string, error: unknown): Promise<void> {
    const o = this.state.get(id);
    this.state.change(id, (op) => {
      op.error = errorText(error);
      op.phase = op.transferIntent ? 'recovery-required' : 'failed';
    });
    if (!o.transferIntent) {
      await this.host.close(o.successorSessionId).catch(() => {});
      await this.release(id, o.sourceSessionId);
    } else await this.host.pause(o.successorSessionId).catch(() => {});
  }
  private async release(id: string, target: string): Promise<boolean> {
    for (const d of this.state.get(id).deliveries.filter((d) => d.kind === 'message')) {
      if (!(await this.deliver(id, d.id, target))) return false;
    }
    return true;
  }
  private async cancel(id: string): Promise<void> {
    const o = this.state.get(id);
    if (o.transferIntent)
      throw new Error(
        'Ownership transfer already began. Reconcile the successor instead of cancelling.',
      );
    if (o.deliveries.some((d) => d.status === 'uncertain' || d.status === 'sending'))
      throw new Error('Resolve uncertain delivery before cancelling');
    this.state.change(id, (op) => {
      op.phase = 'cancelled';
      op.error = 'Handoff cancelled; old manager retained';
    });
    await this.host.close(o.successorSessionId).catch(() => {});
    await this.release(id, o.sourceSessionId);
  }
  private async bind(id: string): Promise<void> {
    const o = this.state.get(id);
    if (!o.committed) throw new Error('Ownership is not committed');
    if (!this.host.bound(o.paneId, o.successorSessionId))
      throw new Error('Waiting for the same pane to attach to the successor');
    if (!o.bound)
      this.state.change(id, (op) => {
        op.bound = true;
      });
    if (o.phase === 'binding') this.run(id, () => this.activate(id));
  }
  private async activate(id: string): Promise<void> {
    let o = this.state.get(id);
    if (!o.bound || o.deliveries.some((d) => d.status === 'uncertain')) return;
    await this.host.validateSuccessor(o.successorSessionId, o.launch);
    this.state.change(id, (op) => {
      op.phase = 'activating';
    });
    const kickoff =
      o.deliveries.find((d) => d.kind === 'kickoff')?.id ??
      this.state.delivery(id, 'kickoff', this.host.kickoff(o));
    if (!(await this.deliver(id, kickoff, o.successorSessionId))) return;
    if (!(await this.release(id, o.successorSessionId))) return;
    o = this.state.get(id);
    await this.host.close(o.sourceSessionId);
    this.state.change(id, (op) => {
      op.retired = true;
      op.phase = 'complete';
      op.error = undefined;
    });
  }
  private async reconcile(id: string): Promise<void> {
    const o = this.state.get(id);
    if (this.running.has(id)) return;
    if (o.transferIntent) {
      await this.host.validateSuccessor(o.successorSessionId, o.launch);
      this.host.transfer(o.sourceSessionId, o.successorSessionId, id);
      this.state.change(id, (op) => {
        op.committed = true;
        op.phase = op.deliveries.some((d) => d.status === 'uncertain')
          ? 'recovery-required'
          : 'binding';
      });
      if (this.state.get(id).bound && this.state.get(id).phase === 'binding')
        this.run(id, () => this.activate(id));
    } else if (!o.deliveries.some((d) => ['uncertain', 'sending'].includes(d.status))) {
      await this.cancel(id); // preparation is never automatically replayed
    }
  }
}
