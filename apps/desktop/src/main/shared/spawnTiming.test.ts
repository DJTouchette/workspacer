import { afterEach, expect, it, vi } from 'vitest';
import { createSpawnTiming } from './spawnTiming';

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it('reports elapsed stages and errors without logging returned values or error payloads', async () => {
  const log = vi.spyOn(console, 'warn').mockImplementation(() => {});
  let now = 100;
  vi.spyOn(performance, 'now').mockImplementation(() => now);
  const timing = createSpawnTiming('claude', 'stream');
  await expect(
    timing.measure('facade_ready', async () => {
      now = 125;
      return 'secret result';
    }),
  ).resolves.toBe('secret result');
  now = 150;
  timing.mark('preflight', 'session-1');
  const failure = new Error('secret token');
  await expect(
    timing.measure('daemon_admission', async () => {
      now = 175;
      throw failure;
    }),
  ).rejects.toBe(failure);
  const records = log.mock.calls.map((call) => JSON.parse(call[1]));
  expect(records.map((r) => [r.stage, r.durationMs, r.elapsedMs, r.outcome])).toEqual([
    ['facade_ready', 25, 25, 'ok'],
    ['preflight', 25, 50, 'ok'],
    ['daemon_admission', 25, 75, 'error'],
  ]);
  expect(new Set(records.map((r) => r.traceId)).size).toBe(1);
  expect(records[1].sessionId).toBe('session-1');
  expect(JSON.stringify(log.mock.calls)).not.toContain('secret');
});

it('works in browser contexts without crypto.randomUUID', () => {
  vi.stubGlobal('crypto', undefined);
  const log = vi.spyOn(console, 'warn').mockImplementation(() => {});
  createSpawnTiming('codex').mark('preflight');
  createSpawnTiming('codex').mark('preflight');
  expect(JSON.parse(log.mock.calls[0][1]).traceId).not.toBe(
    JSON.parse(log.mock.calls[1][1]).traceId,
  );
});
