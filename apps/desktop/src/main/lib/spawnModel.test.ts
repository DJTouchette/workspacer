// THE OMITTED-MODEL DROP, on the desktop side.
//
// `config.claude.defaultModel` ships as `opus[1m]` and SpawnAgentDialog prefills
// the picker from it, so a human clicking Spawn sends an explicit model. Every
// other entry point — App.tsx's restore path, `agents.spawn` over the hub bus
// (the MCP facade, /m, a Fleet Manager dispatching a worker), jobs — left it
// undefined, Claude Code resolved its own default internally, and the daemon
// never learned which one. That is the dispatched-worker case: most of the fleet
// spawned with no recorded request, and the `[1m]` marker is the ONLY carrier of
// a 1M choice before the provider reports a window (Claude Code strips it from
// the id it writes into the transcript).

import { describe, it, expect, vi, beforeEach } from 'vitest';

const getConfig = vi.fn();
vi.mock('../services/configService', () => ({ configService: { getConfig: () => getConfig() } }));

import { resolveSpawnModel } from './spawnModel';

describe('resolveSpawnModel', () => {
  beforeEach(() => {
    getConfig.mockReset();
    getConfig.mockReturnValue({ claude: { defaultModel: 'opus[1m]' } });
  });

  it('fills an omitted claude model from the configured default', () => {
    expect(resolveSpawnModel('claude', undefined)).toBe('opus[1m]');
    expect(resolveSpawnModel('claude', null)).toBe('opus[1m]');
    // Blank and whitespace are omissions, not choices — a dispatch that sent
    // `model: ''` is the same "nobody said" as one that sent nothing.
    expect(resolveSpawnModel('claude', '')).toBe('opus[1m]');
    expect(resolveSpawnModel('claude', '   ')).toBe('opus[1m]');
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
