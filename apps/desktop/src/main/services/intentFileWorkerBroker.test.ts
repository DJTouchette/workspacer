import { EventEmitter } from 'node:events';
import { Worker } from 'node:worker_threads';
import fs from 'node:fs';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { IntentFileWorkerBroker, intentFileWorkerPath } from './intentFileWorkerBroker';
class StubWorker extends EventEmitter {
  messages: unknown[] = [];
  postMessage(message: unknown) {
    this.messages.push(message);
  }
  ref() {
    return this;
  }
  unref() {
    return this;
  }
  terminate = vi.fn(async () => 0);
}
const brokers: IntentFileWorkerBroker[] = [];
afterEach(() => {
  for (const broker of brokers.splice(0)) broker.close();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

it('resolves packaged Electron to the unpacked worker and headless to its sibling asset', () => {
  const root = path.resolve('packaging-fixture');
  const native = path.join(root, 'app.asar', 'dist', 'main', 'services');
  const unpacked = path.join(
    root,
    'app.asar.unpacked',
    'dist',
    'headless',
    'intent-file-worker.cjs',
  );
  const headless = path.join(root, 'hub', 'intent-file-worker.cjs');
  vi.spyOn(fs, 'existsSync').mockImplementation(
    (file) => String(file) === unpacked || String(file) === headless,
  );
  expect(intentFileWorkerPath(native)).toBe(unpacked);
  expect(intentFileWorkerPath(path.join(root, 'hub'))).toBe(headless);
});
function fixture(options: { deadlineMs?: number; maxQueued?: number } = {}) {
  const workers: StubWorker[] = [];
  const broker = new IntentFileWorkerBroker({
    filename: '/host/intent.sqlite',
    ...options,
    createWorker: () => {
      const worker = new StubWorker();
      workers.push(worker);
      return worker as unknown as Worker;
    },
  });
  brokers.push(broker);
  return { broker, workers };
}
const read = { action: 'readArtifact', id: 'workspace', artifactId: 'artifact' };
it('allows only file actions and removes conversations, approvals and arbitrary snapshot fields before cloning', async () => {
  const f = fixture();
  for (const action of [
    'sendDirection',
    'sendControl',
    'publishSourceUpdate',
    'update',
    '__proto__',
  ])
    await expect(f.broker.request({ action })).rejects.toThrow('allowed');
  expect(f.workers).toHaveLength(0);
  const pending = f.broker.request({ action: 'captureEvidence', id: 'work', executionId: 'run' }, [
    {
      sessionId: 'worker',
      cwd: '/host/cwd',
      liveCwd: '/host/tree',
      conversation: [{ role: 'assistant', content: 'large private transcript' }],
      pendingApproval: { toolName: 'shell' },
    },
  ]);
  const message = f.workers[0].messages[0] as {
    token: number;
    sessions: Record<string, unknown>[];
  };
  expect(message.sessions).toEqual([
    {
      sessionId: 'worker',
      cwd: '/host/cwd',
      liveCwd: '/host/tree',
      hub: undefined,
      hubOffline: undefined,
      status: undefined,
      label: undefined,
      provider: undefined,
    },
  ]);
  f.workers[0].emit('message', { token: message.token, error: 'fixture complete' });
  await expect(pending).rejects.toThrow('fixture complete');
});
it('bounds queued work and never dispatches more than one operation at a time', async () => {
  const f = fixture({ maxQueued: 2 });
  const first = f.broker.request(read).catch((e) => e.message);
  const second = f.broker.request(read).catch((e) => e.message);
  const third = f.broker.request(read).catch((e) => e.message);
  await expect(f.broker.request(read)).rejects.toThrow('Too many');
  expect(f.workers[0].messages).toHaveLength(1);
  f.broker.close();
  expect(await first).toContain('closed');
  expect(await second).toContain('before this queued');
  expect(await third).toContain('before this queued');
});
it('counts queue time against the deadline and never replays active or queued work after failure', async () => {
  vi.useFakeTimers();
  const f = fixture({ deadlineMs: 100 });
  const first = f.broker.request(read);
  await vi.advanceTimersByTimeAsync(50);
  const second = f.broker.request(read).catch((e) => e.message);
  await vi.advanceTimersByTimeAsync(40);
  const original = f.workers[0].messages[0] as { token: number };
  f.workers[0].emit('message', {
    token: original.token,
    result: { action: 'readArtifact', artifact: {}, dataBase64: '' },
  });
  await first;
  const queued = f.workers[0].messages[1] as { expiresAt: number };
  expect(queued.expiresAt - Date.now()).toBe(60);
  await vi.advanceTimersByTimeAsync(61);
  expect(await second).toContain('deadline');
  expect(f.workers[0].terminate).toHaveBeenCalledOnce();
  expect(f.workers).toHaveLength(1);
  const explicit = f.broker.request(read).catch((e) => e.message);
  expect(f.workers).toHaveLength(2);
  expect(f.workers[1].messages).toHaveLength(1);
  f.broker.close();
  await explicit;
});
it('keeps the owner event loop responsive while a real worker performs blocking work', async () => {
  const broker = new IntentFileWorkerBroker({
    filename: '/unused',
    deadlineMs: 2000,
    createWorker: () =>
      new Worker(
        `const {parentPort}=require('node:worker_threads');parentPort.on('message',m=>{Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,200);parentPort.postMessage({token:m.token,error:'finished blocking work'});});`,
        { eval: true },
      ),
  });
  brokers.push(broker);
  let finished = false;
  const pending = broker
    .request(read)
    .catch((e) => e.message)
    .finally(() => {
      finished = true;
    });
  await new Promise((resolve) => setTimeout(resolve, 30));
  expect(finished).toBe(false);
  expect(await pending).toBe('finished blocking work');
});
