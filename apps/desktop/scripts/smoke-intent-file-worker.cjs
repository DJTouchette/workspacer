// Run using the installed Electron binary with ELECTRON_RUN_AS_NODE=1. This
// verifies Electron's real WorkerThread/SQLite runtime against the bundled asset.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { Worker } = require('node:worker_threads');
const { DatabaseSync } = require('node:sqlite');
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'intent-electron-worker-'));
const filename = path.join(directory, 'intent.sqlite');
const db = new DatabaseSync(filename);
db.close();
const worker = new Worker(path.resolve('dist/headless/intent-file-worker.cjs'), {
  workerData: { filename },
});
const timer = setTimeout(() => {
  console.error('Electron intent worker did not respond');
  process.exitCode = 1;
  void worker.terminate();
}, 15000);
worker.once('error', (error) => {
  clearTimeout(timer);
  console.error(error);
  process.exitCode = 1;
});
worker.once('message', (reply) => {
  clearTimeout(timer);
  if (reply.token !== 1 || !String(reply.error).includes('Workspace no longer exists')) {
    console.error('Unexpected Electron worker reply', reply);
    process.exitCode = 1;
  } else console.log('Electron WorkerThread loaded bundled intent store and SQLite successfully');
  void worker.terminate();
});
worker.once('exit', () => fs.rmSync(directory, { recursive: true, force: true }));
worker.postMessage({
  token: 1,
  request: { action: 'readArtifact', id: 'missing', artifactId: 'missing' },
  sessions: [],
  expiresAt: Date.now() + 10000,
});
