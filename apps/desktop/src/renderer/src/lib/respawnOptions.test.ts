import { describe, it, expect } from 'vitest';
import { buildRespawnSpawnOptions } from './respawnOptions';
import type { AgentWorkspace } from '../types/pane';

function record(extra: Partial<AgentWorkspace>): AgentWorkspace {
  return {
    id: 'agent-1',
    name: 'Fleet Manager',
    cwd: '/home/u/Work',
    tabs: [],
    activeTabId: 't1',
    ...extra,
  };
}

describe('buildRespawnSpawnOptions — record → respawn round trip', () => {
  it('re-passes the manager role flags so a revived manager re-mints its grants (the regression)', () => {
    const opts = buildRespawnSpawnOptions(
      record({
        provider: 'claude',
        transport: 'stream',
        toolScope: 'operator',
        manager: true,
        fleetFullAccess: true,
        model: 'gpt-5-codex',
        modelIdentity: 'gpt-5-codex',
        contextWindow: 400_000,
        effort: 'xhigh',
        lastSessionId: 'sess-old',
      }),
      'sess-old',
    );

    expect(opts).toMatchObject({
      manager: true,
      fleetFullAccess: true,
      toolScope: 'operator',
      transport: 'stream',
      resumeSessionId: 'sess-old',
      model: 'gpt-5-codex',
      modelIdentity: 'gpt-5-codex',
      contextWindow: 400_000,
      effort: 'xhigh',
    });
  });

  it('revives a card saved under the retired supervisor role at the operator tier', () => {
    // The role is gone, but the saved card still says `supervisor: true` with no
    // toolScope of its own — the role implied one. Without this healing the
    // respawn comes back with no workspacer tools at all, silently.
    const opts = buildRespawnSpawnOptions(
      record({ provider: 'claude', supervisor: true } as never),
      'sess-2',
    );
    expect(opts.toolScope).toBe('operator');
    expect(opts.manager).toBeUndefined();
  });

  it('carries every recorded launch setting through', () => {
    const opts = buildRespawnSpawnOptions(
      record({
        provider: 'claude',
        profileId: 'work',
        model: 'opus[1m]',
        modelIdentity: 'opus',
        contextWindow: 1_000_000,
        effort: 'high',
        permissionMode: 'acceptEdits',
        skipPermissions: false,
        mcpItemIds: ['lib-1'],
        toolScope: 'triage',
        pluginTools: ['p.x'],
      }),
      'sess-3',
    );

    expect(opts).toMatchObject({
      cwd: '/home/u/Work',
      provider: 'claude',
      profileId: 'work',
      model: 'opus[1m]',
      modelIdentity: 'opus',
      contextWindow: 1_000_000,
      effort: 'high',
      permissionMode: 'acceptEdits',
      skipPermissions: false,
      mcpItemIds: ['lib-1'],
      toolScope: 'triage',
      pluginTools: ['p.x'],
      resumeSessionId: 'sess-3',
      cols: 120,
      rows: 32,
    });
  });

  it('preserves a native-1M canonical pair without inventing a marker', () => {
    expect(
      buildRespawnSpawnOptions(
        record({ model: 'fable', modelIdentity: 'fable', contextWindow: 1_000_000 }),
        'native-1m',
      ),
    ).toMatchObject({
      model: 'fable',
      modelIdentity: 'fable',
      contextWindow: 1_000_000,
    });
  });

  it('preserves a marker-bearing compatibility model beside its canonical pair', () => {
    expect(
      buildRespawnSpawnOptions(
        record({ model: 'sonnet[1m]', modelIdentity: 'sonnet', contextWindow: 1_000_000 }),
        'marked-1m',
      ),
    ).toMatchObject({
      model: 'sonnet[1m]',
      modelIdentity: 'sonnet',
      contextWindow: 1_000_000,
    });
  });

  it('keeps the transport rules: claude pty falls back to the config default, stream sticks, managed providers keep theirs', () => {
    // Claude recorded 'pty' (usually just the legacy default) → undefined, so
    // the spawn IPC applies the CURRENT config default transport.
    expect(
      buildRespawnSpawnOptions(record({ provider: 'claude', transport: 'pty' }), 's').transport,
    ).toBeUndefined();
    // Claude explicitly on stream stays stream.
    expect(
      buildRespawnSpawnOptions(record({ provider: 'claude', transport: 'stream' }), 's').transport,
    ).toBe('stream');
    // A managed provider keeps its recorded transport (codex pty = native TUI).
    expect(
      buildRespawnSpawnOptions(record({ provider: 'codex', transport: 'pty' }), 's').transport,
    ).toBe('pty');
  });
});
