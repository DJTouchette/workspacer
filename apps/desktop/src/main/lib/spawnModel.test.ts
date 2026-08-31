// THE OMITTED-MODEL DROP, on the desktop side.
//
// `config.claude.defaultModel` ships as `opus` with `claude.contextWindow: 1000000`,
// and SpawnAgentDialog prefills
// the picker from it, so a human clicking Spawn sends an explicit model. Every
// other entry point — App.tsx's restore path, `agents.spawn` over the hub bus
// (the MCP facade, /m, a Fleet Manager dispatching a worker), jobs — left it
// undefined, Claude Code resolved its own default internally, and the daemon
// never learned which one. That is the dispatched-worker case: most of the fleet
// spawned with no recorded request, and the `[1m]` marker is the ONLY carrier of
// a 1M choice before the provider reports a window (Claude Code strips it from
// the id it writes into the transcript).

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import * as path from 'path';
import { ModelSelectionError } from '../shared/modelContextWindows';

const getConfig = vi.fn();
vi.mock('../services/configService', () => ({ configService: { getConfig: () => getConfig() } }));

import {
  resolveSpawnModel,
  resolveSpawnModelInput,
  resolveSpawnModelSelection,
} from './spawnModel';

interface InputCase {
  name: string;
  provider: string;
  model: string | null;
  modelIdentity: string | null;
  contextWindow: number | null;
  expectedModel: string | null;
  expectedContextWindow: number | null;
  expectedLegacyModel: string | null;
  error: string | null;
  note: string;
}

const fixturePath = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
  '..',
  '..',
  '..',
  '..',
  'contracts',
  'model-context-windows.json',
);
const inputCases = (JSON.parse(readFileSync(fixturePath, 'utf-8')) as { inputCases: InputCase[] })
  .inputCases;

describe('pair-aware spawn input contract (shared with Rust and Go)', () => {
  it('keeps a nontrivial skew/conflict corpus', () => {
    expect(inputCases.length).toBeGreaterThanOrEqual(12);
  });

  it.each(inputCases)('$name', (c) => {
    try {
      const got = resolveSpawnModelInput(c.provider, {
        model: c.model,
        modelIdentity: c.modelIdentity,
        contextWindow: c.contextWindow,
      });
      if (c.error) throw new Error(`expected ${c.error}`);
      expect(got, c.note).toEqual(
        c.expectedModel === null
          ? undefined
          : {
              selection: {
                model: c.expectedModel,
                contextWindow: c.expectedContextWindow,
              },
              legacyModel: c.expectedLegacyModel,
            },
      );
    } catch (err) {
      if (!c.error) throw err;
      expect(err).toBeInstanceOf(ModelSelectionError);
      expect((err as ModelSelectionError).code, c.note).toBe(c.error);
    }
  });
});

describe('resolveSpawnModel', () => {
  beforeEach(() => {
    getConfig.mockReset();
    getConfig.mockReturnValue({ claude: { defaultModel: 'opus', contextWindow: 1_000_000 } });
  });

  it('fills an omitted claude model from the configured default', () => {
    expect(resolveSpawnModel('claude', undefined)).toBe('opus');
    expect(resolveSpawnModel('claude', null)).toBe('opus');
    // Blank and whitespace are omissions, not choices — a dispatch that sent
    // `model: ''` is the same "nobody said" as one that sent nothing.
    expect(resolveSpawnModel('claude', '')).toBe('opus');
    expect(resolveSpawnModel('claude', '   ')).toBe('opus');
    expect(resolveSpawnModelSelection('claude', undefined)).toEqual({
      model: 'opus',
      contextWindow: 1_000_000,
    });
  });

  it('never overrides a caller who named a model', () => {
    expect(resolveSpawnModel('claude', 'sonnet')).toBe('sonnet');
    expect(resolveSpawnModel('claude', '  haiku  ')).toBe('haiku');
  });

  it('leaves other providers alone — only claude has a configured default', () => {
    expect(resolveSpawnModel('codex', undefined)).toBeUndefined();
    expect(resolveSpawnModel('codex', 'gpt-5-codex')).toBe('gpt-5-codex');
    expect(resolveSpawnModel('opencode', undefined)).toBeUndefined();
  });

  it('stays undefined when there is no configured default to fill from', () => {
    // This FILLS A BLANK; it does not invent one. A user who cleared the
    // default gets the CLI's own choice and an honest "we were not told".
    getConfig.mockReturnValue({ claude: { defaultModel: '' } });
    expect(resolveSpawnModel('claude', undefined)).toBeUndefined();
    getConfig.mockReturnValue({});
    expect(resolveSpawnModel('claude', undefined)).toBeUndefined();
  });
});

/**
 * The second drop resolveSpawnModel closes: a model belonging to a DIFFERENT
 * harness. Every spawn funnels through here, so this is the last place a
 * `sonnet` can be stopped before it lands on a codex argv.
 *
 * The callers that make this necessary are the ones that are NOT pickers — the
 * MCP facade's `spawn_agent` (a supervisor relaying a configured summarizer id),
 * a hub job's hand-written spec, a respawn replaying a record written before the
 * agent's provider changed. A picker sourced from the provider's own catalog
 * cannot produce one of these, which is exactly why the bug was invisible.
 */
describe('resolveSpawnModel — cross-harness model ids', () => {
  it('drops a claude id from a codex spawn and uses codex’s own default', () => {
    getConfig.mockReturnValue({ claude: { defaultModel: 'opus', contextWindow: 1_000_000 } });
    expect(resolveSpawnModel('codex', 'sonnet')).toBeUndefined();
    expect(resolveSpawnModel('codex', 'haiku')).toBeUndefined();
    expect(resolveSpawnModel('opencode', 'fable')).toBeUndefined();
  });

  it('drops a codex id from a claude spawn — and does NOT substitute claude.defaultModel', () => {
    // Substituting the config default here would be worse than the harness's
    // own: it would silently run a DIFFERENT model than the caller named while
    // reporting success. Undefined means "the CLI's default", which is honest.
    getConfig.mockReturnValue({ claude: { defaultModel: 'opus', contextWindow: 1_000_000 } });
    expect(resolveSpawnModel('claude', 'gpt-5.1-codex-max')).toBeUndefined();
  });

  it('still forwards a model the provider DOES serve', () => {
    getConfig.mockReturnValue({ claude: { defaultModel: 'opus', contextWindow: 1_000_000 } });
    expect(resolveSpawnModel('claude', 'sonnet')).toBe('sonnet');
    expect(resolveSpawnModel('codex', 'gpt-5.1-codex-max')).toBe('gpt-5.1-codex-max');
    expect(resolveSpawnModel('opencode', 'anthropic/claude-sonnet-4')).toBe(
      'anthropic/claude-sonnet-4',
    );
  });

  it('still forwards an id no harness claims — a private deployment is a real choice', () => {
    getConfig.mockReturnValue({ claude: {} });
    expect(resolveSpawnModel('codex', 'my-finetune-v3')).toBe('my-finetune-v3');
  });

  it.each([
    ['codex', 'gpt-5-codex-1m'],
    ['opencode', 'openrouter/custom-1m'],
    ['pi', 'pi-local-1m'],
  ])('%s keeps a non-Claude -1m model id byte-for-byte unchanged', (provider, model) => {
    expect(resolveSpawnModel(provider, model)).toBe(model);
    expect(resolveSpawnModelSelection(provider, model)).toEqual({ model, contextWindow: null });
  });

  it('still normalizes Claude legacy ids into a canonical model/window pair', () => {
    expect(resolveSpawnModel('claude', 'opus-1m')).toBe('opus');
    expect(resolveSpawnModelSelection('claude', 'opus-1m')).toEqual({
      model: 'opus',
      contextWindow: 1_000_000,
    });
  });
});
