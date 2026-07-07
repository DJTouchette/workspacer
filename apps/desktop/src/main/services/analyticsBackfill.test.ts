import { describe, it, expect, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// The backfill module pulls in the sqlite singleton (Electron-native) and the
// history store at module scope — stub both; these tests only exercise the
// pure transcript-recompute path.
vi.mock('./db', () => ({ database: { get db() { throw new Error('db not used'); } } }));
vi.mock('./sessionHistory', () => ({ sessionHistory: { recordModels: vi.fn() } }));

import { recomputeSession } from './analyticsBackfill';

function writeTranscript(rows: object[]): string {
  const file = path.join(
    fs.mkdtempSync(path.join(os.tmpdir(), 'wks-backfill-')),
    'session.jsonl',
  );
  fs.writeFileSync(file, rows.map((r) => JSON.stringify(r)).join('\n') + '\n');
  return file;
}

const assistant = (
  id: string,
  model: string,
  usage: object,
  extra: object = {},
): object => ({
  type: 'assistant',
  message: { id, model, role: 'assistant', usage, content: [] },
  ...extra,
});

describe('recomputeSession', () => {
  it('returns null for a transcript with no assistant usage', async () => {
    const file = writeTranscript([
      { type: 'user', message: { role: 'user', content: 'hi' } },
      { type: 'summary', summary: 'x' },
    ]);
    expect(await recomputeSession(file)).toBeNull();
  });

  it('accumulates main-thread usage at current model rates', async () => {
    const file = writeTranscript([
      assistant('m1', 'claude-opus-4-8', { input_tokens: 1_000_000, output_tokens: 1_000_000 }),
    ]);
    const re = await recomputeSession(file);
    expect(re).toMatchObject({
      model: 'claude-opus-4-8',
      inputTokens: 1_000_000,
      outputTokens: 1_000_000,
      peakContext: 1_000_000,
    });
    // opus 4.8: $5/M in + $25/M out = $30
    expect(re!.costUSD).toBeCloseTo(30, 6);
  });

  it('counts sidechain turns into totals/split but not context or model', async () => {
    const file = writeTranscript([
      assistant('m1', 'claude-fable-5', { input_tokens: 1_000, output_tokens: 500 }),
      assistant(
        'sub1',
        'claude-haiku-4-5',
        { input_tokens: 1_000_000, output_tokens: 1_000_000 },
        { isSidechain: true },
      ),
    ]);
    const re = await recomputeSession(file);
    expect(re!.model).toBe('claude-fable-5');
    // sidechain must not move the gauge
    expect(re!.peakContext).toBe(1_000);
    expect(re!.inputTokens).toBe(1_001_000);
    expect(re!.outputTokens).toBe(1_000_500);
    // fable (1k in @$10 + 500 out @$50)/1e6 + haiku 1M/1M at $1/$5
    expect(re!.costUSD).toBeCloseTo(0.035 + 6, 6);
    expect(re!.models['claude-fable-5'].costUSD).toBeCloseTo(0.035, 9);
    expect(re!.models['claude-haiku-4-5'].costUSD).toBeCloseTo(6, 9);
  });

  it('dedups repeated message ids (streamed blocks re-list the message)', async () => {
    const file = writeTranscript([
      assistant('m1', 'claude-sonnet-4-5', { input_tokens: 100, output_tokens: 10 }),
      assistant('m1', 'claude-sonnet-4-5', { input_tokens: 100, output_tokens: 10 }),
      assistant('m2', 'claude-sonnet-4-5', { input_tokens: 200, output_tokens: 20 }),
    ]);
    const re = await recomputeSession(file);
    expect(re!.inputTokens).toBe(300);
    expect(re!.outputTokens).toBe(30);
  });

  it('treats "<synthetic>" as unnamed, inheriting the thread model', async () => {
    const file = writeTranscript([
      assistant('m1', 'claude-opus-4-8', { input_tokens: 100, output_tokens: 10 }),
      assistant('m2', '<synthetic>', { input_tokens: 0, output_tokens: 1 }),
    ]);
    const re = await recomputeSession(file);
    expect(re!.model).toBe('claude-opus-4-8');
    expect(Object.keys(re!.models)).toEqual(['claude-opus-4-8']);
  });

  it('survives malformed lines without dropping the rest', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-backfill-'));
    const file = path.join(dir, 'session.jsonl');
    fs.writeFileSync(
      file,
      'not json at all\n' +
        JSON.stringify(
          assistant('m1', 'claude-haiku-4-5', { input_tokens: 50, output_tokens: 5 }),
        ) +
        '\n{truncated',
    );
    const re = await recomputeSession(file);
    expect(re!.inputTokens).toBe(50);
  });
});

describe('recomputeSession — separate subagent transcript files', () => {
  it('folds subagents/*.jsonl as sidechain spend', async () => {
    // Current Claude Code layout: sub-agent rows live in their own files, not
    // interleaved in the main transcript (and regardless of their own
    // isSidechain flags, everything in those files is sub-agent spend).
    const main = writeTranscript([
      assistant('m1', 'claude-fable-5', { input_tokens: 1_000, output_tokens: 500 }),
    ]);
    const sub = writeTranscript([
      assistant('s1', 'claude-opus-4-8', { input_tokens: 1_000_000, output_tokens: 0 }),
      assistant('s1', 'claude-opus-4-8', { input_tokens: 1_000_000, output_tokens: 0 }), // dup id
      assistant('s2', 'claude-haiku-4-5', { input_tokens: 0, output_tokens: 1_000_000 }),
    ]);
    const re = await recomputeSession(main, [sub]);
    expect(re!.model).toBe('claude-fable-5');
    expect(re!.peakContext).toBe(1_000);
    expect(re!.inputTokens).toBe(1_001_000);
    expect(re!.outputTokens).toBe(1_000_500);
    // fable main + opus 1M in ($5) + haiku 1M out ($5)
    expect(re!.costUSD).toBeCloseTo(0.035 + 5 + 5, 6);
    expect(Object.keys(re!.models).sort()).toEqual([
      'claude-fable-5',
      'claude-haiku-4-5',
      'claude-opus-4-8',
    ]);
  });

  it('still returns usage when only subagent files carry any', async () => {
    const main = writeTranscript([{ type: 'user', message: { role: 'user', content: 'hi' } }]);
    const sub = writeTranscript([
      assistant('s1', 'claude-haiku-4-5', { input_tokens: 100, output_tokens: 10 }),
    ]);
    const re = await recomputeSession(main, [sub]);
    expect(re).not.toBeNull();
    expect(re!.inputTokens).toBe(100);
    expect(re!.model).toBeNull();
    expect(re!.peakContext).toBe(0);
  });
});
