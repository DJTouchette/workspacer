import { describe, it, expect, vi, beforeEach } from 'vitest';

// Back the module's rates file with an in-memory blob so we can exercise the
// override path without touching the real ~/.workspacer/model-rates.json.
let fileContent: string | null = null; // null = file absent
let fileMtime = 1;

vi.mock('fs', () => ({
  default: {
    statSync: () => {
      if (fileContent === null) throw new Error('ENOENT');
      return { mtimeMs: fileMtime };
    },
    readFileSync: () => {
      if (fileContent === null) throw new Error('ENOENT');
      return fileContent;
    },
    mkdirSync: () => {},
    rmSync: () => {
      fileContent = null;
      fileMtime += 1;
    },
  },
}));
vi.mock('os', () => ({ default: { homedir: () => '/home/test' } }));
vi.mock('../lib/atomicWriteFile', () => ({
  atomicWriteFileSync: (_p: string, data: string) => {
    fileContent = data;
    fileMtime += 1;
  },
}));

import {
  turnCostUSD,
  contextLimitFor,
  readModelRateOverrides,
  writeModelRateOverrides,
} from './modelUsage';

describe('model rate overrides', () => {
  beforeEach(() => {
    writeModelRateOverrides({}); // clears the file + the mtime cache
  });

  it('falls back to built-in rates when there is no override', () => {
    expect(turnCostUSD('claude-fable-5', { input_tokens: 1_000_000 })).toBe(10);
    expect(contextLimitFor('claude-fable-5', 0)).toBe(1_000_000); // 1M-native
  });

  it('an override re-rates both cost and context window', () => {
    writeModelRateOverrides({
      'claude-fable': { input: 1, output: 2, context_limit: 1_000_000 },
    });
    expect(turnCostUSD('claude-fable-5', { input_tokens: 1_000_000 })).toBe(1);
    expect(turnCostUSD('claude-fable-5', { output_tokens: 1_000_000 })).toBe(2);
    expect(contextLimitFor('claude-fable-5', 0)).toBe(1_000_000);
    expect(readModelRateOverrides()['claude-fable'].input).toBe(1);
  });

  it('a same-length override beats the built-in of the same prefix', () => {
    writeModelRateOverrides({ 'claude-opus': { input: 7, output: 70 } });
    // 'claude-opus-4-8-…' matches generic 'claude-opus'; override wins the tie.
    expect(turnCostUSD('claude-opus-4-8-20260101', { input_tokens: 1_000_000 })).toBe(7);
  });

  it('reset (empty map) reverts to built-ins', () => {
    writeModelRateOverrides({ 'claude-fable': { input: 1, output: 2 } });
    expect(turnCostUSD('claude-fable-5', { input_tokens: 1_000_000 })).toBe(1);
    writeModelRateOverrides({});
    expect(turnCostUSD('claude-fable-5', { input_tokens: 1_000_000 })).toBe(10);
    expect(readModelRateOverrides()).toEqual({});
  });
});
