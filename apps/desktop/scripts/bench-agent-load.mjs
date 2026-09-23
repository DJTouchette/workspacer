// Synthetic hot-path benchmark, not an Electron or provider startup profile.
// Uses production code and real disk writes in a disposable directory. No
// providers are launched and no personal configuration/history is accessed.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { performance } from 'node:perf_hooks';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-agent-load-'));
const require = createRequire(import.meta.url);
const observations = 120;
const counters = { reads: 0, writes: 0, fsyncs: 0, locks: 0 };
const originals = new Map();
const reset = () => Object.keys(counters).forEach((key) => (counters[key] = 0));
const round = (n) => Math.round(n * 100) / 100;

try {
  const bundle = path.join(temp, 'subject.cjs');
  await build({
    stdin: {
      contents: `
        export { DispatchHistoryStore } from './src/main/services/dispatchHistoryStore';
        export { compactClaudeSnapshotForBackground } from './src/main/shared/compactClaudeSnapshot';
      `,
      resolveDir: root,
      loader: 'ts',
    },
    outfile: bundle,
    bundle: true,
    platform: 'node',
    format: 'cjs',
    plugins: [
      {
        name: 'isolate-personal-config',
        setup(b) {
          b.onResolve({ filter: /\/configService$/ }, () => ({
            path: 'config',
            namespace: 'bench',
          }));
          b.onLoad({ filter: /.*/, namespace: 'bench' }, () => ({
            contents:
              'export function getConfigDir() { throw new Error("Benchmark must not access personal config"); }',
            loader: 'js',
          }));
        },
      },
    ],
  });
  // Patch before loading the bundled subject, including its namespace imports.
  for (const [method, counter] of [
    ['readFileSync', 'reads'],
    ['writeFileSync', 'writes'],
    ['fsyncSync', 'fsyncs'],
    ['openSync', 'locks'],
  ]) {
    const original = fs[method];
    originals.set(method, original);
    fs[method] = function (...args) {
      if (method !== 'openSync' || String(args[0]).endsWith('.lock')) counters[counter]++;
      return original.apply(this, args);
    };
  }
  const { DispatchHistoryStore, compactClaudeSnapshotForBackground } = require(bundle);
  const history = [];
  for (const tasks of [1, 50, 200]) {
    const filename = path.join(temp, `history-${tasks}.json`);
    const store = new DispatchHistoryStore(() => filename);
    for (let i = 0; i < tasks; i++) {
      store.accept({
        owner: { sessionId: 'manager', isWakeTarget: true, status: 'active' },
        projectCwd: temp,
        executionCwd: temp,
        sessionId: `worker-${i}`,
        title: `Benchmark task ${i}`,
      });
      store.observe({
        sessionId: `worker-${i}`,
        status: 'active',
        ambientState: 'streaming',
        pendingApproval: null,
      });
    }
    for (const mode of ['immediate', 'batched']) {
      for (const kind of ['tracked', 'untracked', 'remote']) {
        reset();
        const start = performance.now();
        for (let i = 0; i < observations; i++) {
          store[mode === 'batched' ? 'queueObservation' : 'observe']({
            sessionId: kind === 'tracked' ? `worker-${i % tasks}` : 'untracked',
            status: 'active',
            ambientState: 'streaming',
            pendingApproval: null,
            pendingQuestions: null,
            statusLine: { totalOutputTokens: i + 1 },
            ...(kind === 'remote' ? { hub: 'peer' } : {}),
          });
        }
        // Include the actual disk commit in the batching result. Remote queue
        // observations are intentionally ignored and schedule no flush.
        if (mode === 'batched' && kind !== 'remote') store.flush();
        const elapsed = performance.now() - start;
        history.push({
          tasks,
          mode,
          kind,
          observations,
          totalMs: round(elapsed),
          msPerUpdate: round(elapsed / observations),
          ...counters,
        });
      }
    }
  }

  const snapshots = [];
  for (const turns of [12, 500, 2000]) {
    const full = {
      sessionId: 'synthetic',
      conversation: Array.from({ length: turns }, (_, i) => ({
        role: i % 2 ? 'assistant' : 'user',
        content: `${i}: ${'x'.repeat(2000)}`,
      })),
      activeToolCalls: [],
      completedToolCalls: [],
      fileChanges: [],
    };
    const compact = compactClaudeSnapshotForBackground(full);
    for (const [shape, snapshot] of [
      ['full', full],
      ['compact', compact],
    ]) {
      // Node structuredClone is a serialization-cost proxy, not Electron IPC.
      structuredClone(snapshot);
      const start = performance.now();
      for (let i = 0; i < 60; i++) structuredClone(snapshot);
      snapshots.push({
        turns,
        shape,
        jsonBytes: Buffer.byteLength(JSON.stringify(snapshot)),
        cloneMsPerUpdate: round((performance.now() - start) / 60),
      });
    }
  }
  console.log(
    JSON.stringify(
      {
        note: 'Synthetic production-code benchmark. Timings depend on host and temp filesystem; clone timings are not Electron IPC measurements.',
        node: process.version,
        platform: process.platform,
        history,
        snapshots,
      },
      null,
      2,
    ),
  );
} finally {
  for (const [method, original] of originals) fs[method] = original;
  fs.rmSync(temp, { recursive: true, force: true });
}
