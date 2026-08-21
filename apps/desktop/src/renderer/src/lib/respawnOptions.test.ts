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
    });
    expect(opts.supervisor).toBeUndefined();
  });

  it('re-passes the supervisor flag', () => {
    const opts = buildRespawnSpawnOptions(
      record({ provider: 'claude', supervisor: true, toolScope: 'operator' }),
      'sess-2',
    );
    expect(opts.supervisor).toBe(true);
    expect(opts.manager).toBeUndefined();
  });

  it('carries every recorded launch setting through', () => {
    const opts = buildRespawnSpawnOptions(
      record({
        provider: 'claude',
        profileId: 'work',
        model: 'opus',
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
      model: 'opus',
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
