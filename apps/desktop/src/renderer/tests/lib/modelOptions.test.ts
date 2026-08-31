import { describe, expect, it } from 'vitest';
import {
  claudeCatalogOptions,
  modelOptionCommand,
  modelOptionMatches,
  rememberedClaudeModelPatch,
} from '../../src/lib/modelOptions';

describe('Claude model picker options', () => {
  it('accepts legacy value-only rows without undefined commands', () => {
    const options = claudeCatalogOptions({ aliases: [{ value: 'opus', label: 'Opus' }] });

    expect(options).toHaveLength(1);
    expect(options[0].id).toBe('opus');
    expect(modelOptionCommand(options[0])).toBe('opus');
  });

  it('uses model plus contextWindow as identity and serializes only at selection', () => {
    const options = claudeCatalogOptions({
      aliases: [
        { model: 'opus', value: 'opus', label: 'Opus', contextWindow: 200_000 },
        { model: 'opus', value: 'opus[1m]', label: 'Opus', contextWindow: 1_000_000 },
      ],
    });

    expect(options.map((option) => option.key)).toHaveLength(
      new Set(options.map((o) => o.key)).size,
    );
    expect(options.map(modelOptionCommand)).toEqual(['opus', 'opus[1m]']);
    expect(modelOptionMatches(options[0], 'opus', 200_000)).toBe(true);
    expect(modelOptionMatches(options[1], 'opus', 200_000)).toBe(false);
    expect(modelOptionMatches(options[1], 'opus[1m]')).toBe(true);
  });

  it('chooses the base row for a bare alias without making explicit 1M ambiguous', () => {
    const options = claudeCatalogOptions({
      aliases: [
        { model: 'opus', value: 'opus', label: 'Opus', contextWindow: 200_000 },
        { model: 'opus', value: 'opus[1m]', label: 'Opus', contextWindow: 1_000_000 },
        { model: 'fable', value: 'fable', label: 'Fable', contextWindow: 1_000_000 },
      ],
    });

    expect(modelOptionMatches(options[0], 'opus')).toBe(true);
    expect(modelOptionMatches(options[1], 'opus')).toBe(false);
    expect(modelOptionMatches(options[0], 'opus[1m]')).toBe(false);
    expect(modelOptionMatches(options[1], 'opus[1m]')).toBe(true);
    expect(modelOptionMatches(options[2], 'fable')).toBe(true);
  });

  it('does not rewrite the saved pair for a prompt-first spawn with no explicit model', () => {
    const saved = { defaultModel: 'opus', contextWindow: 1_000_000 };
    const promptFirstPatch = rememberedClaudeModelPatch(undefined);
    expect(promptFirstPatch).toEqual({});
    expect({ ...saved, ...promptFirstPatch }).toEqual(saved);
    expect(rememberedClaudeModelPatch('opus[1m]')).toEqual({
      defaultModel: 'opus',
      contextWindow: 1_000_000,
    });
    expect(rememberedClaudeModelPatch('fable')).toEqual({
      defaultModel: 'fable',
      contextWindow: 1_000_000,
    });
  });
});
