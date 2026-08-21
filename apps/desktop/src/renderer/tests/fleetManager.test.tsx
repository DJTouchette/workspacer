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
import { buildManagerKickoff, deriveFleetRoot, FLEET_MANAGER_NAME } from '../src/lib/fleetManager';

describe('buildManagerKickoff — full-access mode', () => {
  it('adds the full-access note only when the flag is set', () => {
    expect(buildManagerKickoff('go', false)).not.toContain('FULL-ACCESS MODE IS ON');
    const yolo = buildManagerKickoff('go', true);
    expect(yolo).toContain('FULL-ACCESS MODE IS ON');
    expect(yolo).toContain('will not stop for approval');
    // The ask still lands at the end after the mode note.
    expect(yolo.trimEnd().endsWith('go')).toBe(true);
  });
});

describe('deriveFleetRoot', () => {
  it('explicit config wins', () => {
    expect(deriveFleetRoot('/srv/code', ['/home/u/Work/a'], '/home/u')).toBe('/srv/code');
  });
  it('derives the common parent of the configured projects', () => {
    expect(deriveFleetRoot('', ['/home/u/Work/alpha', '/home/u/Work/beta/nested'], '/home/u')).toBe(
      '/home/u/Work',
    );
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
    // The manager keeps its OWN brief too — its memory across restarts, held
    // to cross-project state only (mirroring project briefs is the drift
    // failure mode the doctrine forbids).
    expect(kickoff).toContain('YOUR OWN fleet brief');
    expect(kickoff).toContain('memory across restarts');
    expect(kickoff).toContain('ONLY cross-project state');
    // First-run managers looped guessing MCP argument names — the kickoff now
    // carries exact call shapes (arg names must stay in lockstep with the
    // facade's input structs in services/hub/cmd/mcp/main.go).
    expect(kickoff).toContain('TOOL SYNTAX');
    expect(kickoff).toContain('"parentSessionId"');
    expect(kickoff).toContain('"sinceSeq"');
    expect(kickoff).toContain('"decision":"yes"');
    // …and model economics: cheap models for chores, frontier only when earned.
    expect(kickoff).toContain('list_models');
    expect(kickoff).toContain('haiku-class');
    expect(kickoff).toContain('Never burn a frontier model on a chore');
    // The anti-poll rule must be a hard STOP, not a soft "stay idle" — a
    // monitoring loop hangs the manager and locks the user out (the reported bug).
    expect(kickoff).toContain('NEVER POLL');
    expect(kickoff).toContain('end your turn');
    // The visible-terminal path: bring up a dev server the user can watch.
    expect(kickoff).toContain('open_terminal');
    expect(kickoff).toContain('does NOT block your turn');
    // Harness pool: dispatch on codex/opencode/pi, not just claude.
    expect(kickoff).toContain('list_providers');
    expect(kickoff).toContain('codex');
    // Ship vs scout task shapes + worktree isolation + per-project delivery.
    expect(kickoff).toContain('SHIP task');
    expect(kickoff).toContain('SCOUT task');
    expect(kickoff).toContain('worktree":true');
    expect(kickoff).toContain('DELIVERY MODE');
    expect(kickoff).toContain('projects[<dir>].delivery');
    // Per-project yolo autonomy.
    expect(kickoff).toContain('projects[<dir>].yolo');
    // The invocable skills the manager is told it has.
    expect(kickoff).toContain('/standup');
    expect(kickoff).toContain('/checkpoint');
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
