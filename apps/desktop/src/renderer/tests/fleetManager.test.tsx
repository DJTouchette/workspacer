/**
 * Fleet Manager (FLEET_MANAGER_SPIKE.md): the spawn contract, reuse-by-name,
 * and the fleet-root derivation. The manager's whole design rides on three
 * facts pinned here: it spawns chat-first at operator tier with the manager
 * flag (nudge routing), its kickoff is AUTO-SENT doctrine (never a composer
 * pre-fill), and a live manager is reused — a second ask must not mint a
 * second manager.
 */
import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useAgentManager } from '../src/hooks/useAgentManager';
import {
  buildManagerKickoff,
  deriveFleetRoot,
  FLEET_MANAGER_NAME,
} from '../src/lib/fleetManager';

describe('deriveFleetRoot', () => {
  it('explicit config wins', () => {
    expect(deriveFleetRoot('/srv/code', ['/home/u/Work/a'], '/home/u')).toBe('/srv/code');
  });
  it('derives the common parent of the configured projects', () => {
    expect(
      deriveFleetRoot('', ['/home/u/Work/alpha', '/home/u/Work/beta/nested'], '/home/u'),
    ).toBe('/home/u/Work');
  });
  it('a lone project pins its PARENT (the project is one level below the root)', () => {
    expect(deriveFleetRoot('', ['/home/u/Work/alpha'], '/home/u')).toBe('/home/u/Work');
  });
  it('falls back to home when projects share no meaningful parent', () => {
    expect(deriveFleetRoot('', ['/srv/x', '/opt/y'], '/home/u')).toBe('/home/u');
    expect(deriveFleetRoot('', [], '/home/u')).toBe('/home/u');
  });
});

describe('spawnFleetManager', () => {
  const spawnClaude = window.electronAPI.spawnClaude as Mock;
  const claudeMessage = window.electronAPI.claudeMessage as Mock;

  beforeEach(() => {
    spawnClaude.mockReset();
    spawnClaude.mockResolvedValue('mgr-session');
    claudeMessage.mockReset();
    claudeMessage.mockResolvedValue(undefined);
  });

  it('spawns chat-first at operator tier with the manager flag and an auto-sent kickoff', async () => {
    const hook = renderHook(() => useAgentManager());
    await act(async () => {
      await hook.result.current.spawnFleetManager('status please', '/home/u/Work');
    });
    expect(spawnClaude).toHaveBeenCalledTimes(1);
    const opts = spawnClaude.mock.calls[0][0];
    expect(opts).toMatchObject({
      cwd: '/home/u/Work',
      transport: 'stream',
      toolScope: 'operator',
      manager: true,
    });
    // The kickoff is the doctrine + the ask, auto-sent (kickoffMessage), and
    // says the load-bearing words.
    expect(claudeMessage).toHaveBeenCalledWith('mgr-session', buildManagerKickoff('status please'));
    const kickoff = buildManagerKickoff('status please');
    expect(kickoff).toContain('You DELEGATE');
    expect(kickoff).toContain('.workspacer/brief.md');
    expect(kickoff).toContain('parentSessionId');
    hook.unmount();
  });

  it('reuses a LIVE manager by name — the ask goes as a plain message, no second spawn', async () => {
    const hook = renderHook(() => useAgentManager());
    act(() =>
      hook.result.current.loadAgentsFromSession(
        [
          {
            id: 'agent-m',
            name: FLEET_MANAGER_NAME,
            cwd: '/home/u/Work',
            sessionId: 'mgr-live',
            tabs: [
              {
                id: 't',
                title: 'M',
                panes: [{ id: 'p', type: 'claude', title: 'C', attachSessionId: 'mgr-live' }],
                activePaneId: 'p',
              },
            ],
            activeTabId: 't',
          } as any,
        ],
        'agent-m',
      ),
    );
    await act(async () => {
      await hook.result.current.spawnFleetManager('and now?', '/home/u/Work');
    });
    expect(spawnClaude).not.toHaveBeenCalled();
    await waitFor(() => expect(claudeMessage).toHaveBeenCalledWith('mgr-live', 'and now?'));
    hook.unmount();
  });
});
