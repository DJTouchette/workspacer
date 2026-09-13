import fs from 'node:fs';
import path from 'node:path';
import { Worker } from 'node:worker_threads';
import type { IntentLiveSession, IntentWorkspaceResponse } from '../shared/intentWorkspace';
import {
  INTENT_FILE_DEADLINE_MS,
  intentFileSessions,
  intentFileRequest,
  isIntentFileAction,
  type IntentFileWorkerRequest,
  type IntentFileWorkerResponse,
} from './intentFileWorkerProtocol';

export function intentFileWorkerPath(directory = __dirname): string {
  const candidates = [
    path.join(directory, 'intent-file-worker.cjs'),
    path.resolve(directory, '../../headless/intent-file-worker.cjs'),
  ];
  for (const candidate of candidates) {
    const unpacked = candidate.replace(/\.asar([\\/])/g, '.asar.unpacked$1');
    if (fs.existsSync(unpacked)) return unpacked;
  }
  throw new Error('Intent file worker is missing. Rebuild or update this host.');
}
type Pending = IntentFileWorkerRequest & {
  resolve: (result: IntentWorkspaceResponse) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
};
export class IntentFileWorkerBroker {
  private worker?: Worker;
  private queue: Pending[] = [];
  private active?: Pending;
  private token = 0;
  private closed = false;
  constructor(
    private readonly options: {
      filename: string;
      workerPath?: string;
      deadlineMs?: number;
      maxQueued?: number;
      createWorker?: () => Worker;
    },
  ) {}
  request(
    request: Record<string, unknown>,
    sessions: readonly IntentLiveSession[] = [],
  ): Promise<IntentWorkspaceResponse> {
    if (!isIntentFileAction(request))
      return Promise.reject(new Error('Action is not an allowed intent file operation'));
    try {
      request = intentFileRequest(request);
    } catch (error) {
      return Promise.reject(error);
    }
    if (this.closed) return Promise.reject(new Error('Intent file worker is closed'));
    if (this.queue.length >= (this.options.maxQueued ?? 8))
      return Promise.reject(
        new Error(
          'Too many intent file requests are queued. Try again after the current operation finishes.',
        ),
      );
    const duration = Math.min(
      this.options.deadlineMs ?? INTENT_FILE_DEADLINE_MS,
      INTENT_FILE_DEADLINE_MS,
    );
    return new Promise((resolve, reject) => {
      const entry: Pending = {
        token: ++this.token,
        request,
        sessions: request.action === 'captureEvidence' ? intentFileSessions(sessions) : [],
        expiresAt: Date.now() + duration,
        resolve,
        reject,
        timer: setTimeout(() => this.expire(entry), duration),
      };
      this.queue.push(entry);
      this.drain();
    });
  }
  private start(): Worker {
    if (this.worker) return this.worker;
    const worker =
      this.options.createWorker?.() ??
      new Worker(this.options.workerPath ?? intentFileWorkerPath(), {
        workerData: { filename: this.options.filename },
      });
    this.worker = worker;
    worker.on('message', (response: IntentFileWorkerResponse) => {
      if (worker !== this.worker || response?.token !== this.active?.token) return;
      const entry = this.active!;
      this.active = undefined;
      clearTimeout(entry.timer);
      if ('error' in response) entry.reject(new Error(response.error));
      else entry.resolve(response.result);
      this.drain();
    });
    worker.on('error', () => this.fail(worker));
    worker.on('exit', () => this.fail(worker));
    return worker;
  }
  private drain(): void {
    if (this.active || !this.queue.length) {
      if (!this.active) this.worker?.unref();
      return;
    }
    this.active = this.queue.shift()!;
    try {
      const worker = this.start();
      worker.ref();
      const { token, request, sessions, expiresAt } = this.active;
      worker.postMessage({ token, request, sessions, expiresAt });
    } catch {
      this.fail(this.worker);
    }
  }
  private expire(entry: Pending): void {
    if (this.active === entry) {
      this.fail(
        this.worker,
        'Intent file operation exceeded its deadline. Its outcome may be uncertain; inspect saved records before trying another write.',
      );
      return;
    }
    const index = this.queue.indexOf(entry);
    if (index >= 0) {
      this.queue.splice(index, 1);
      entry.reject(
        new Error(
          'Queued intent file request expired before it started. No file operation was attempted.',
        ),
      );
    }
  }
  private fail(
    worker: Worker | undefined,
    detail = 'Intent file worker stopped. The active operation may have completed; inspect saved records before trying another write.',
  ): void {
    if (worker && worker !== this.worker) return;
    this.worker = undefined;
    const pending = [this.active, ...this.queue];
    this.active = undefined;
    this.queue = [];
    for (const entry of pending)
      if (entry) {
        clearTimeout(entry.timer);
        entry.reject(
          new Error(
            entry === pending[0]
              ? detail
              : 'Intent file worker stopped before this queued operation started. No file operation was attempted.',
          ),
        );
      }
    void worker?.terminate();
  }
  close(): void {
    this.closed = true;
    this.fail(
      this.worker,
      'Intent file worker closed. Inspect saved records before retrying an active write.',
    );
  }
}
let broker: IntentFileWorkerBroker | undefined;
export function runIntentFileRequest(
  filename: string,
  request: Record<string, unknown>,
  sessions: readonly IntentLiveSession[],
): Promise<IntentWorkspaceResponse> {
  broker ??= new IntentFileWorkerBroker({ filename });
  return broker.request(request, sessions);
}
