import path from 'path';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import {
  ManagerDeliveryRejected,
  ManagerReplacementUnavailable,
  ManagerOwnershipUnchanged,
} from '../shared/managerReplacement';
import { randomUUID } from 'crypto';
import {
  ManagerReplacementState,
  savedFinishDelivery,
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
  projects?(id: string): string[];
  readyForTransfer(id: string): boolean;
  signatures(ids: string[]): Record<string, string>;
  inFlightMessages?(
    id: string,
  ): Array<{
    id: string;
    text: string;
    signatures?: Array<[string, string]>;
    sourceRequest?: import('../shared/managerReplacement').ReplacementDelivery['sourceRequest'];
  }>;
  finishes(id: string): Record<string, { reply: string; stopped: boolean }>;
  receipt(id: string): string;
  settled(id: string): boolean;
  spawn(id: string, launch: ManagerLaunch): Promise<void>;
  validateSuccessor(id: string, launch: ManagerLaunch): Promise<void>;
  transfer(source: string, successor: string, operationId: string): void;
  restore(metadata: ReplacementMetadata[]): Promise<void>;
  bound(paneId: string, id: string): boolean;
  send(
    id: string,
    text: string,
    sourceRequest?: import('../shared/managerReplacement').ReplacementDelivery['sourceRequest'],
  ): Promise<{ ok: boolean; mode?: string }>;
  pause(id: string): Promise<void>;
  close(id: string): Promise<void>;
  kickoff(op: ReplacementRecord): string;
  recoverFinishes(op: ReplacementRecord): void;
  flushFinishes?(parentIds: string[]): Promise<void>;
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
    this.state.enableProjection();
    return (this.initialized ??= this.recover().catch((error) => {
      this.initialized = undefined;
      throw error;
    }));
  }
  private async recover(): Promise<void> {
    await this.bounded(
      'restoring manager metadata',
      this.host.restore(this.state.recoveryMetadata()),
    );
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
          o.bound = false;
          o.error =
            'Desktop restarted during handoff. Inspect retained delivery evidence; nothing was replayed.';
        }
      });
      const o = this.state.get(id);
      // Restore metadata without inventing liveness, before releasing any gate.
      await this.bounded(
        'restoring journal metadata',
        this.host.restore(
          o.metadata.map((m) => ({
            ...m,
            ...(m.parentSessionId === o.sourceSessionId && (o.committed || o.transferIntent)
              ? { parentSessionId: o.successorSessionId }
              : {}),
          })),
        ),
      );
      if (o.transferIntent && o.phase !== 'complete') await this.reconcile(id);
      else if (o.phase === 'recovery-required') {
        await this.bounded(
          'pausing uncertain owner',
          this.host.pause(o.committed ? o.successorSessionId : o.sourceSessionId),
        ).catch(() => {});
      }
      this.host.recoverFinishes(this.state.get(id));
    }
  }
  async request(request: ManagerReplacementRequest): Promise<ManagerReplacementResponse> {
    try {
      if (
        !request ||
        !['list', 'start', 'cancel', 'bind', 'reconcile', 'resolve-delivery'].includes(
          request.action,
        ) ||
        JSON.stringify(request).length > 4096
      )
        throw new Error('Invalid manager replacement request');
      if (
        request.action === 'resolve-delivery' &&
        !['retry', 'accepted'].includes(request.resolution)
      )
        throw new Error('Invalid delivery resolution');
      await this.initialize();
      if (['start', 'cancel', 'reconcile', 'resolve-delivery'].includes(request.action))
        this.state.lastError = undefined;
      if (request.action === 'start')
        this.start(request.sourceSessionId, request.paneId, request.workspaceId);
      else if (request.action === 'cancel') await this.cancel(request.operationId);
      else if (request.action === 'reconcile') await this.reconcile(request.operationId);
      else if (request.action === 'bind') await this.bind(request.operationId);
      else if (request.action === 'resolve-delivery') {
        if (request.acknowledgeDuplicateRisk !== true)
          throw new Error('Explicit acknowledgement is required: retry may duplicate agent work');
        const op = this.state.get(request.operationId);
        if (this.running.has(op.operationId))
          throw new Error(
            'Wait for the current handoff action to settle before resolving delivery',
          );
        let delivery = op.deliveries.find((d) => d.id === request.deliveryId);
        if (!delivery && request.deliveryId.startsWith('finish:')) {
          const worker = request.deliveryId.slice('finish:'.length);
          const evidence = op.finishes[worker];
          if (evidence) {
            delivery = savedFinishDelivery(worker, evidence);
            this.state.change(op.operationId, (o) => {
              o.deliveries.push(delivery!);
              delete o.finishes[worker];
            });
          }
        }
        if (!delivery || !['uncertain', 'pending'].includes(delivery.status))
          throw new Error('Delivery is not awaiting resolution');
        const original = delivery;
        if (original.sourceRequest && request.resolution === 'retry')
          throw new Error(
            'Resolve this authoritative inbox request instead. Handoff does not replay captured user requests.',
          );
        let nextId = original.id;
        this.state.change(op.operationId, (o) => {
          const d = o.deliveries.find((d) => d.id === original.id)!;
          d.status = 'reconciled';
          d.error = [
            d.error,
            request.resolution === 'accepted'
              ? 'User inspected and accounted for this delivery'
              : 'User explicitly authorized another attempt; this original request may still arrive',
          ]
            .filter(Boolean)
            .join('\n');
          if (request.resolution === 'retry') {
            nextId = randomUUID();
            o.deliveries.push({
              id: nextId,
              kind: d.kind,
              text: d.text,
              status: 'pending',
              error: `Explicit retry of ${d.id}; may duplicate work`,
            });
          }
        });
        delivery = this.state.get(op.operationId).deliveries.find((d) => d.id === nextId)!;
        if (request.resolution === 'retry')
          await this.deliver(
            op.operationId,
            nextId,
            op.committed ? op.successorSessionId : op.sourceSessionId,
          );
        if (
          !op.transferIntent &&
          delivery.kind === 'preparation' &&
          this.state.get(op.operationId).deliveries.find((d) => d.id === delivery.id)?.status !==
            'uncertain'
        ) {
          this.state.change(op.operationId, (o) => {
            o.phase = 'preparing';
            o.error = undefined;
          });
          this.run(op.operationId, () => this.prepare(op.operationId));
        } else await this.reconcile(op.operationId);
      }
      return { available: true, operations: this.state.views(), error: this.state.lastError };
    } catch (error) {
      return {
        available: !(error instanceof ManagerReplacementUnavailable),
        operations: this.safeViews(),
        error: errorText(error),
      };
    }
  }
  private safeViews() {
    try {
      return this.state.views();
    } catch {
      return [];
    }
  }
  start(sourceSessionId: string, paneId: string, workspaceId: string): string {
    if (
      !sourceSessionId ||
      !paneId ||
      !workspaceId ||
      sourceSessionId.length > 128 ||
      paneId.length > 256
    )
      throw new Error('Source session and owning pane are required');
    const previous = this.state
      .records()
      .find(
        (o) =>
          o.sourceSessionId === sourceSessionId &&
          (!['failed', 'cancelled'].includes(o.phase) ||
            o.deliveries.some((d) => d.status === 'uncertain' || d.status === 'pending')),
      );
    if (previous) {
      if (previous.paneId !== paneId)
        throw new Error(
          `Manager handoff ${previous.operationId} belongs to another pane; inspect its owning pane`,
        );
      return previous.operationId;
    }
    if (this.state.records().length >= 32)
      throw new Error(
        'Replacement journal capacity reached; existing audit and ownership metadata was retained; use the standalone handoff fallback',
      );
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
      for (const m of metadata) if (d.pendingMetadata) delete d.pendingMetadata[m.sessionId];
      d.operations.push({
        operationId,
        sourceSessionId,
        successorSessionId: randomUUID(),
        paneId,
        workspaceId,
        phase: 'preparing',
        createdAt: now,
        updatedAt: now,
        committed: false,
        bound: false,
        artifactPath,
        workerIds,
        taskIds,
        deliveries: (this.host.inFlightMessages?.(sourceSessionId) ?? []).map((frame) => ({
          id: frame.id,
          kind: 'message',
          text: frame.sourceRequest ? '' : frame.text,
          sourceRequest: frame.sourceRequest,
          status: 'sending',
        })),
        launch,
        projectCwds: this.host.projects?.(sourceSessionId) ?? [],
        metadata,
        signatures: {
          ...this.host.signatures(workerIds),
          ...Object.fromEntries(
            (this.host.inFlightMessages?.(sourceSessionId) ?? []).flatMap(
              (f) => f.signatures ?? [],
            ),
          ),
        },
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
      .catch((error) => {
        this.state.lastError = errorText(error);
      })
      .finally(() => this.running.delete(id));
    this.running.set(id, run);
  }
  async idle(id: string): Promise<void> {
    await this.running.get(id);
  }
  private async bounded<T>(stage: string, action: Promise<T>): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([
        action,
        new Promise<never>((_, reject) => {
          timer = setTimeout(
            () => reject(new Error(`Timed out ${stage}. Inspect handoff state before retrying.`)),
            this.timing.deliveryMs,
          );
        }),
      ]);
    } finally {
      if (timer) clearTimeout(timer);
    }
  }
  private async prepare(id: string): Promise<void> {
    const deadline = Date.now() + this.timing.preparationMs;
    while (
      this.state.activeCount(this.state.get(id).sourceSessionId) ||
      !this.host.readyForTransfer(this.state.get(id).sourceSessionId)
    ) {
      if (Date.now() >= deadline)
        throw new Error('Pending dispatch did not settle before handoff timeout');
      await wait(this.timing.pollMs);
    }
    let o = this.state.get(id);
    if (o.phase !== 'preparing') return;
    await this.bounded(
      'draining earlier completion delivery',
      this.host.flushFinishes?.([o.sourceSessionId]) ?? Promise.resolve(),
    );
    o = this.state.get(id);
    if (o.phase !== 'preparing') {
      if (o.phase === 'recovery-required')
        await this.bounded('pausing predecessor', this.host.pause(o.sourceSessionId)).catch(
          () => {},
        );
      return;
    }
    if (this.state.lastError) throw new Error(this.state.lastError);
    // Include dispatches accepted before the fence, even if their registration
    // or task-history acceptance completed after the click.
    const metadata = this.host.inventory(o.sourceSessionId);
    this.state.change(id, (op) => {
      op.metadata = metadata;
      op.workerIds = metadata
        .filter((m) => m.parentSessionId === op.sourceSessionId)
        .map((m) => m.sessionId);
      op.taskIds = this.host.tasks(op.sourceSessionId);
      op.projectCwds = this.host.projects?.(op.sourceSessionId) ?? [];
    });
    o = this.state.get(id);
    const preparation =
      o.deliveries.find((d) => d.kind === 'preparation')?.id ??
      this.state.delivery(id, 'preparation', preparationPrompt(o));
    if (!(await this.deliver(id, preparation, o.sourceSessionId))) return;
    let artifact: { raw: string; hash: string } | undefined;
    while (Date.now() < deadline) {
      o = this.state.get(id);
      if (o.phase !== 'preparing') return;
      const receipt = this.host.receipt(o.sourceSessionId);
      if (
        this.host.settled(o.sourceSessionId) &&
        receipt.includes('```wks-manager-handoff') &&
        receipt.includes(id)
      ) {
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
    if (!this.host.readyForTransfer(o.sourceSessionId))
      throw new Error('A task dispatch reservation is still active; ownership was not transferred');
    const currentWorkers = this.host
      .inventory(o.sourceSessionId)
      .filter((m) => m.parentSessionId === o.sourceSessionId)
      .map((m) => m.sessionId);
    if (!same(currentWorkers, o.workerIds) || !same(this.host.tasks(o.sourceSessionId), o.taskIds))
      throw new Error(
        'Owned workers or tasks changed during preparation; retry with a fresh checkpoint',
      );
    // Keep the validated bytes separate from the agent's proposal path, so a
    // late duplicate preparation turn cannot rewrite the successor's input.
    createArtifactPath(o.launch.options.cwd!, id);
    const sealedArtifactPath = path.join(path.dirname(o.artifactPath), 'validated-handoff.json');
    atomicWriteFileSync(sealedArtifactPath, artifact.raw, { mode: 0o600 });
    this.state.change(id, (op) => {
      op.sealedArtifactPath = sealedArtifactPath;
      op.artifact = artifact!.raw;
      op.artifactHash = artifact!.hash;
      op.phase = 'spawning';
    });
    let spawnTimer: ReturnType<typeof setTimeout> | undefined;
    const spawned = this.host.spawn(o.successorSessionId, o.launch);
    // A late spawn still belongs to this cancelled/failed operation, never to
    // a retry. It had no kickoff; close only this pinned candidate if it lands.
    void spawned
      .then(() => {
        if (
          ![
            'spawning',
            'transferring',
            'binding',
            'activating',
            'complete',
            'recovery-required',
          ].includes(this.state.get(id).phase)
        )
          return this.host.close(o.successorSessionId);
      })
      .catch(() => {});
    try {
      await Promise.race([
        spawned,
        new Promise<never>((_, reject) => {
          spawnTimer = setTimeout(
            () =>
              reject(
                new Error(
                  'Successor spawn timed out. Old manager retained; the pinned late candidate will be closed without a kickoff.',
                ),
              ),
            this.timing.deliveryMs,
          );
        }),
      ]);
    } finally {
      if (spawnTimer) clearTimeout(spawnTimer);
    }
    if (this.state.get(id).phase !== 'spawning') {
      await this.host.close(o.successorSessionId);
      return;
    }
    await this.bounded(
      'verifying successor',
      this.host.validateSuccessor(o.successorSessionId, o.launch),
    );
    if (this.state.get(id).phase !== 'spawning') {
      await this.host.close(o.successorSessionId);
      return;
    }
    if (
      JSON.stringify(this.host.source(o.sourceSessionId, o.paneId)) !== JSON.stringify(o.launch) ||
      !this.host.readyForTransfer(o.sourceSessionId) ||
      !same(this.host.tasks(o.sourceSessionId), o.taskIds) ||
      !same(
        this.host
          .inventory(o.sourceSessionId)
          .filter((m) => m.parentSessionId === o.sourceSessionId)
          .map((m) => m.sessionId),
        o.workerIds,
      )
    )
      throw new Error(
        'Source identity, grants or fleet changed before ownership commit; old manager retained',
      );
    this.state.change(id, (op) => {
      if (!op.metadata.some((m) => m.sessionId === op.successorSessionId))
        op.metadata.push({
          sessionId: op.successorSessionId,
          cwd: op.launch.options.cwd!,
          provider: op.launch.options.provider,
          isWakeTarget: true,
          label: op.launch.options.label ?? 'Fleet Manager',
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
    const op = this.state.get(id);
    if (
      delivery.kind === 'message' &&
      !delivery.sourceRequest &&
      op.committed &&
      target === op.successorSessionId
    ) {
      const correction = `\n\n[Host manager handoff ${id}] Current manager/parentSessionId is ${op.successorSessionId}. Earlier owner IDs in quoted worker instructions or workflow hints refer to the predecessor. Preserve task IDs and pinned policy; call next_workflow_step for current workflow state before any continuation. Do not adopt again or replay a worker task.`;
      if (!delivery.text.includes(correction))
        this.state.change(id, (o) => {
          o.deliveries.find((d) => d.id === deliveryId)!.text += correction;
        });
    }
    this.state.change(id, (o) => {
      o.deliveries.find((d) => d.id === deliveryId)!.status = 'sending';
    });
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = Date.now() + this.timing.deliveryMs;
    const sendWhenReady = async () => {
      while (true) {
        if (Date.now() >= deadline) throw new ManagerDeliveryRejected(404);
        try {
          return await this.host.send(target, delivery.text, delivery.sourceRequest);
        } catch (error) {
          if (
            delivery.sourceRequest ||
            !(error instanceof ManagerDeliveryRejected) ||
            error.status !== 404 ||
            Date.now() + this.timing.pollMs >= deadline
          )
            throw error;
          await wait(this.timing.pollMs);
        }
      }
    };
    try {
      const result = await Promise.race([
        sendWhenReady(),
        new Promise<never>((_, reject) => {
          timer = setTimeout(
            () => reject(new Error('Timed out awaiting daemon acknowledgement')),
            this.timing.deliveryMs,
          );
        }),
      ]);
      if (!result.ok) throw new ManagerDeliveryRejected(409);
      this.state.change(id, (o) => {
        o.deliveries.find((d) => d.id === deliveryId)!.status = 'accepted';
      });
      return true;
    } catch (error) {
      this.state.change(id, (o) => {
        const d = o.deliveries.find((d) => d.id === deliveryId)!;
        d.status = error instanceof ManagerDeliveryRejected ? 'pending' : 'uncertain';
        d.error = errorText(error);
        o.phase = 'recovery-required';
        o.error =
          error instanceof ManagerDeliveryRejected
            ? 'The daemon explicitly rejected the message. It remains queued; inspect the destination and retry when ready.'
            : 'Delivery acknowledgement is uncertain. Inspect the destination before explicitly retrying; retry can duplicate work.';
      });
      // Best effort only: providers may already have acted. The host dispatch
      // fence remains authoritative even if interruption fails.
      if (!(error instanceof ManagerDeliveryRejected))
        await this.bounded('pausing uncertain delivery', this.host.pause(target)).catch(() => {});
      return false;
    } finally {
      if (timer) clearTimeout(timer);
    }
  }
  private async fail(id: string, error: unknown): Promise<void> {
    const o = this.state.get(id);
    const uncertain = o.deliveries.some((d) => d.status === 'sending' || d.status === 'uncertain');
    this.state.change(id, (op) => {
      op.error = errorText(error);
      if (error instanceof ManagerOwnershipUnchanged) op.transferIntent = false;
      for (const d of op.deliveries)
        if (d.status === 'sending') {
          d.status = 'uncertain';
          d.error = errorText(error);
        }
      op.phase = op.transferIntent || uncertain ? 'recovery-required' : 'failed';
    });
    if (!o.transferIntent) {
      await this.bounded('closing parked successor', this.host.close(o.successorSessionId)).catch(
        () => {},
      );
      if (uncertain)
        await this.bounded('pausing predecessor', this.host.pause(o.sourceSessionId)).catch(
          () => {},
        );
      await this.release(id, o.sourceSessionId);
    } else
      await this.bounded('pausing successor', this.host.pause(o.successorSessionId)).catch(
        () => {},
      );
  }
  private async release(id: string, target: string): Promise<boolean> {
    // Re-read after every await: finishes can arrive while another held wake
    // is being acknowledged. Never mark complete with an unsent queued item.
    while (true) {
      const d = this.state
        .get(id)
        .deliveries.find(
          (d) => d.kind === 'message' && !['accepted', 'reconciled'].includes(d.status),
        );
      if (!d) return true;
      if (!(await this.deliver(id, d.id, target))) return false;
    }
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
    await this.bounded('closing parked successor', this.host.close(o.successorSessionId)).catch(
      () => {},
    );
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
    await this.bounded(
      'verifying successor',
      this.host.validateSuccessor(o.successorSessionId, o.launch),
    );
    this.state.change(id, (op) => {
      op.phase = 'activating';
    });
    const kickoff =
      o.deliveries.find((d) => d.kind === 'kickoff')?.id ??
      this.state.delivery(id, 'kickoff', this.host.kickoff(o));
    if (!(await this.deliver(id, kickoff, o.successorSessionId))) return;
    await this.bounded(
      'draining completion wakes',
      this.host.flushFinishes?.([o.sourceSessionId, o.successorSessionId]) ?? Promise.resolve(),
    );
    if (!(await this.release(id, o.successorSessionId))) return;
    o = this.state.get(id);
    if (this.state.lastError) throw new Error(this.state.lastError);
    if (Object.keys(this.state.get(id).finishes).length)
      throw new Error(
        'Recorded completions remain undelivered. Inspect their retained evidence before continuing.',
      );
    await this.bounded('retiring predecessor', this.host.close(o.sourceSessionId));
    await this.bounded(
      'draining completion wakes',
      this.host.flushFinishes?.([o.sourceSessionId, o.successorSessionId]) ?? Promise.resolve(),
    );
    if (!(await this.release(id, o.successorSessionId))) return;
    this.state.change(id, (op) => {
      if (Object.keys(op.finishes).length)
        throw new Error('Recorded completions remain undelivered; inspect retained evidence');
      op.retired = true;
      op.phase = 'complete';
      op.error = undefined;
    });
  }
  private async reconcile(id: string): Promise<void> {
    const o = this.state.get(id);
    if (this.running.has(id)) return;
    if (o.transferIntent) {
      await this.bounded(
        'verifying successor',
        this.host.validateSuccessor(o.successorSessionId, o.launch),
      );
      this.host.transfer(o.sourceSessionId, o.successorSessionId, id);
      this.host.recoverFinishes(this.state.get(id));
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
