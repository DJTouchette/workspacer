import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { DatabaseSync } from 'node:sqlite';
import { buildSync } from 'esbuild';
import { afterAll, beforeAll, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({
  getConfigDir: () => path.join(os.tmpdir(), 'intent-worker-config'),
}));
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import { IntentFileWorkerBroker } from './intentFileWorkerBroker';
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'intent-worker-'));
const workerPath = path.join(directory, 'intent-file-worker.cjs');
const db = new DatabaseSync(path.join(directory, 'work.sqlite'));
const store = new IntentWorkspaceStore(db);
const broker = new IntentFileWorkerBroker({
  filename: path.join(directory, 'work.sqlite'),
  workerPath,
});
beforeAll(() =>
  buildSync({
    entryPoints: ['src/main/services/intentFileWorker.ts'],
    outfile: workerPath,
    bundle: true,
    platform: 'node',
    format: 'cjs',
    target: 'node22',
  }),
);
afterAll(() => {
  broker.close();
  db.close();
});
function workspace() {
  const result = store.request({
    action: 'create',
    projectRoot: directory,
    fields: {
      title: 'Worker work',
      outcome: '',
      constraints: '',
      successCriteria: 'Retained bytes',
      sourceUrl: '',
      status: 'active',
    },
  });
  if (result.action !== 'create') throw new Error('Expected workspace');
  return result.workspace;
}
it('uses the production bundled worker and its own SQLite connection to retain bytes visible immediately to the owner', async () => {
  const work = workspace();
  const input = {
    action: 'addArtifact',
    id: work.id,
    expectedRevision: 1,
    artifactId: 'worker-artifact',
    title: 'Worker bytes',
    mimeType: 'text/plain',
    dataBase64: Buffer.from('saved off owner loop').toString('base64'),
  };
  expect(await broker.request(input)).toMatchObject({
    action: 'addArtifact',
    artifact: { id: input.artifactId },
  });
  expect(store.request({ action: 'artifacts', id: work.id })).toMatchObject({
    artifacts: [expect.objectContaining({ id: input.artifactId })],
  });
  expect(
    await broker.request({ action: 'readArtifact', id: work.id, artifactId: input.artifactId }),
  ).toMatchObject({ dataBase64: input.dataBase64 });
  expect(await broker.request(input)).toMatchObject({
    action: 'addArtifact',
    artifact: { id: input.artifactId },
  });
}, 60000);
it('does not allow messaging or source publishing through the production worker boundary', async () => {
  await expect(broker.request({ action: 'sendControl' })).rejects.toThrow('allowed');
  await expect(broker.request({ action: 'publishSourceUpdate' })).rejects.toThrow('allowed');
});
it.runIf(process.platform === 'win32')(
  'keeps owner timers and metadata writes responsive during real Windows native helper I/O',
  async () => {
    const work = workspace();
    await broker.request({ action: 'knowledge', id: work.id }); // Warm the worker, not PowerShell.
    let settled = false;
    const pending = broker
      .request({
        action: 'addArtifact',
        id: work.id,
        expectedRevision: 1,
        artifactId: 'concurrent-windows',
        title: 'Concurrent',
        mimeType: 'text/plain',
        dataBase64: Buffer.from('bytes').toString('base64'),
      })
      .then(
        (r) => ({ result: r }),
        (e: Error) => ({ error: e.message }),
      )
      .finally(() => {
        settled = true;
      });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(settled).toBe(false);
    const started = Date.now();
    store.request({
      action: 'update',
      id: work.id,
      expectedRevision: 1,
      fields: { ...work, title: 'Owner remained responsive' },
      reason: 'Concurrent owner edit',
    });
    expect(Date.now() - started).toBeLessThan(1000);
    expect(await pending).toMatchObject({ error: expect.stringContaining('changed elsewhere') });
    expect(store.request({ action: 'artifacts', id: work.id })).toMatchObject({ artifacts: [] });
  },
  60000,
);
