/**
 * `supervisor.model` is one field for three harnesses, and a model id is never
 * portable between them. The rules pinned here are the whole reason
 * lib/supervisorModel exists rather than an inline `supCfg?.model` read:
 *
 *  - a per-harness choice (supervisor.models[provider]) always wins;
 *  - the legacy single `supervisor.model` applies ONLY to the harness it was
 *    chosen on (`supervisor.provider`) — handing `fable` to a codex spawn, or
 *    `gpt-5-codex` to a Claude one, is a 400 at spawn, not a fallback;
 *  - anything unresolved is undefined, i.e. "let the CLI pick its own default",
 *    which is the only value that is valid on every harness.
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';

let mockConfig: Record<string, unknown> = {};
vi.mock('../services/configService', () => ({
  configService: { getConfig: () => mockConfig },
}));

import { resolveSupervisorModel } from './supervisorModel';

beforeEach(() => {
  mockConfig = {};
});

describe('resolveSupervisorModel', () => {
  it('returns the per-harness choice for that harness', () => {
    mockConfig = {
      supervisor: {
        provider: 'codex',
        model: 'gpt-5-codex',
        models: { codex: 'gpt-5-codex', claude: 'fable' },
      },
    };
    expect(resolveSupervisorModel('codex')).toBe('gpt-5-codex');
    expect(resolveSupervisorModel('claude')).toBe('fable');
  });

  it('falls back to the legacy single field only for the configured harness', () => {
    mockConfig = { supervisor: { provider: 'codex', model: 'gpt-5-codex' } };
    expect(resolveSupervisorModel('codex')).toBe('gpt-5-codex');
    // THE BUG THIS CLOSES: a Claude supervisor launched from "Ask the Fleet"
    // while the configured harness is codex used to inherit the codex id.
    expect(resolveSupervisorModel('claude')).toBeUndefined();
  });

  it('treats an absent supervisor.provider as claude', () => {
    mockConfig = { supervisor: { model: 'fable' } };
    expect(resolveSupervisorModel('claude')).toBe('fable');
    expect(resolveSupervisorModel('codex')).toBeUndefined();
  });

  it('is undefined when nothing is configured, so the CLI picks its own default', () => {
    expect(resolveSupervisorModel('claude')).toBeUndefined();
    expect(resolveSupervisorModel('codex')).toBeUndefined();
    mockConfig = { supervisor: { provider: 'claude', model: '   ', models: { codex: '' } } };
    expect(resolveSupervisorModel('claude')).toBeUndefined();
    expect(resolveSupervisorModel('codex')).toBeUndefined();
  });
});
