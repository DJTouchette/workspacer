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

describe('buildManagerKickoff — requested selection readback', () => {
  const doctrine = buildManagerKickoff('status');

  it('points the manager at the canonical config tuple and distinguishes runtime truth', () => {
    expect(doctrine).toContain('get_config');
    expect(doctrine).toContain('agents.managerContextWindows');
    expect(doctrine).toMatch(/requested[^]*effective/i);
    expect(doctrine).toContain('runtime truth');
    expect(doctrine).toContain('Never use the requested value as a context-bar denominator');
  });
});

describe('buildManagerKickoff — concise operating contract', () => {
  const doctrine = buildManagerKickoff('go');
  it('bounds instructions and uses event-driven, incremental reads', () => {
    expect(doctrine.length).toBeLessThan(12000);
    expect(doctrine.endsWith('The user says:\n\ngo')).toBe(true);
    for (const term of [
      'NEVER POLL',
      'After dispatch end your turn',
      'list_manager_requests({view:"pending"})',
      'contentDeferred',
      'remaining > 0',
      'userContent remains user input',
      'Do not fetch the same evidence again',
      'lastMessage:true',
      'compact:true',
      'no startup sweep',
    ])
      expect(doctrine).toContain(term);
  });
  it('preserves routing, ownership, admission and paired-path constraints', () => {
    for (const term of [
      'SELECT_MODEL FIRST',
      'eligible:false',
      'escalationScrubbed',
      'parentSessionId',
      'taskId',
      'afterDispatchId',
      'workflowStepId',
      'decisionId',
      'list_dispatches',
      'select_dispatch_model',
      'remoteCwd',
      'Never raise capability',
      'Never repeat a spawn whose admission is unknown',
    ])
      expect(doctrine).toContain(term);
  });
  it('preserves independent review and evidence acceptance', () => {
    for (const term of [
      'fresh DIFFERENT worker',
      'acceptance criteria',
      'architectural constraints',
      'test results',
      'previousProvider',
      'targeted independent validation',
      'accept_task_outcome',
      "Never provide the implementer's reasoning, plan or transcript",
      'Followup readiness grants no execution or publish authority',
      'Do not treat idle, a valid schema, waived steps or terminal policy as success',
    ])
      expect(doctrine).toContain(term);
  });
  it('preserves standalone versus host-owned recovery', () => {
    for (const term of [
      'HOST-OWNED MANAGER HANDOFF takes precedence',
      'Do not adopt, read/delete shared handoff.md',
      'list_orphans',
      'confirmedManager:false is not proof',
      'adopt_workers fromSessionId',
    ])
      expect(doctrine).toContain(term);
  });
  it('preserves templates, escalation validation and existing authorization', () => {
    for (const term of [
      'DISPATCH TEMPLATES',
      'templateParams',
      'default resultSchema',
      'Required placeholders must be filled',
      'wks-escalation',
      'worker-escalated',
      'type,status,reason,requiredAuthorityOrDecision,changed,nextAction',
      'Malformed escalation blocks remain ordinary prose',
      'never waive requested result validation',
      'Existing explicit authorization persists',
    ])
      expect(doctrine).toContain(term);
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
  // The break this pins: agents.fleetRoot typed as '~/' in Settings reached the
  // spawn as a directory literally named '~' (nothing downstream expands it, by
  // binding decision), so every Fleet Manager came up stopped-on-arrival — a
  // card that opens into an empty session and answers no messages.
  it("expands a leading '~' in the explicit root — a person typing '~/' means home", () => {
    expect(deriveFleetRoot('~/', [], '/home/u')).toBe('/home/u/');
    expect(deriveFleetRoot('~', [], '/home/u')).toBe('/home/u');
    expect(deriveFleetRoot('~/Work', [], '/home/u')).toBe('/home/u/Work');
    expect(deriveFleetRoot('  ~/Work  ', [], '/home/u')).toBe('/home/u/Work');
  });
  it("leaves '~user' and mid-path tildes alone — only the home shorthand is ours to resolve", () => {
    expect(deriveFleetRoot('~alice/Work', [], '/home/u')).toBe('~alice/Work');
    expect(deriveFleetRoot('/srv/a~b', [], '/home/u')).toBe('/srv/a~b');
  });
  it('expands the same shorthand in project paths before taking their common parent', () => {
    expect(deriveFleetRoot('', ['~/Work/alpha', '~/Work/beta'], '/home/u')).toBe('/home/u/Work');
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

  // The delivery mechanism itself, stated once as its own case: a kickoff is
  // carried BY the spawn, never fired at the session afterwards. The two-call
  // form is what left a manager live with no doctrine when its provider driver
  // had not come up yet.
  it('never fires a separate send after the spawn — the kickoff rides the payload', async () => {
    const hook = renderHook(() => useAgentManager());
    await act(async () => {
      await hook.result.current.spawnFleetManager('status please', '/home/u/Work');
    });
    expect(spawnClaude.mock.calls[0][0].message).toBeTruthy();
    expect(claudeMessage).not.toHaveBeenCalled();
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
    // The kickoff is the doctrine + the ask, auto-sent — and it rides the SPAWN
    // (`message`) rather than a separate send fired the moment spawnClaude
    // resolves. That send raced the session coming up: a manager on the stream
    // transport is a managed row, registered with no prompt channel yet, and
    // claudemon refuses a message in that window with a 404 — a Fleet Manager
    // sitting there with no doctrine and no ask.
    expect(opts.message).toBe(buildManagerKickoff('status please'));
    expect(claudeMessage).not.toHaveBeenCalled();
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
    // Argument details live in schemas; keep the operating rules in the role.
    expect(kickoff).toContain('parentSessionId');
    expect(kickoff).toContain('sinceSeq');
    // …and model economics, which is now a routing call rather than a habit:
    // the matrix answers which model a role is worth, and the manager copies it.
    expect(kickoff).toContain('select_model');
    expect(kickoff).toContain('capability');
    expect(kickoff).toContain('escalationScrubbed');
    // The anti-poll rule must be a hard STOP, not a soft "stay idle" — a
    // monitoring loop hangs the manager and locks the user out (the reported bug).
    expect(kickoff).toContain('NEVER POLL');
    expect(kickoff).toContain('end your turn');
    for (const term of [
      'open_terminal',
      'list_providers',
      'code-intelligence tools',
      'the host delivers that automatically',
      'SHIP changes',
      'SCOUT/REVIEW',
      'worktree:true',
      'projects[<dir>].delivery',
      'yolo:true',
      '/standup',
      '/checkpoint',
      '/handoff',
      '.workspacer/handoff.md',
      'archiving Recently overflow',
      'User=fleet preferences',
    ])
      expect(kickoff).toContain(term);
    hook.unmount();
  });

  it('runs the manager on the configured harness, role flags intact', async () => {
    // The Overview entry point used to hardcode provider 'claude', so a Fleet
    // Manager on codex was impossible. The role flags matter more than the
    // provider: without `manager` the session is never marked isWakeTarget and
    // NO worker-finished wake is routed to it, and without `fleetFullAccess`
    // its token is minted with no dispatch grants.
    const hook = renderHook(() => useAgentManager());
    await act(async () => {
      await hook.result.current.spawnFleetManager('status', '/home/u/Work', false, true, 'codex');
    });
    expect(spawnClaude).toHaveBeenCalledTimes(1);
    expect(spawnClaude.mock.calls[0][0]).toMatchObject({
      provider: 'codex',
      transport: 'stream',
      toolScope: 'operator',
      manager: true,
      fleetFullAccess: true,
    });
    hook.unmount();
  });

  it('carries the configured manager model through to the spawn payload', async () => {
    // `agents.managerProvider` shipped with no model twin, so the manager
    // always ran on its harness's default. The renderer passes the resolved
    // per-harness value so it lands on the AGENT RECORD (the card, the pill and
    // every later restart read it there); main re-resolves the same value from
    // live config for the entry points that never come through here.
    const hook = renderHook(() => useAgentManager());
    await act(async () => {
      await hook.result.current.spawnFleetManager(
        'status',
        '/home/u/Work',
        false,
        false,
        'codex',
        'gpt-5-codex',
      );
    });
    expect(spawnClaude.mock.calls[0][0]).toMatchObject({
      provider: 'codex',
      model: 'gpt-5-codex',
      manager: true,
    });
    hook.unmount();
  });

  it('records the requested manager context on the fresh manager card/spawn', async () => {
    const hook = renderHook(() => useAgentManager());
    await act(async () => {
      await hook.result.current.spawnFleetManager(
        'status',
        '/home/u/Work',
        false,
        false,
        'codex',
        'gpt-5-codex',
        1_000_000,
        'xhigh',
      );
    });
    expect(spawnClaude.mock.calls[0][0]).toMatchObject({
      provider: 'codex',
      model: 'gpt-5-codex',
      contextWindow: 1_000_000,
      effort: 'xhigh',
      manager: true,
    });
    expect(
      hook.result.current.agents.find((agent) => agent.name === FLEET_MANAGER_NAME),
    ).toMatchObject({
      provider: 'codex',
      model: 'gpt-5-codex',
      contextWindow: 1_000_000,
      effort: 'xhigh',
    });
    hook.unmount();
  });

  it('sends no model at all when none is configured — the harness defaults', async () => {
    // Undefined, NOT an empty string: main's resolveSpawnModel treats a blank
    // as "no model named" and falls through to the harness's own default, which
    // is the one value valid on every harness. An '' would be indistinguishable
    // from a real choice at a glance and is the kind of value that ends up on
    // an argv.
    const hook = renderHook(() => useAgentManager());
    await act(async () => {
      await hook.result.current.spawnFleetManager('status', '/home/u/Work', false, false, 'codex');
    });
    expect(spawnClaude.mock.calls[0][0].model).toBeUndefined();
    hook.unmount();
  });

  it('does not resurrect a stopped manager from a DIFFERENT harness', async () => {
    // A conversation cannot move between harnesses, so after switching
    // agents.managerProvider the stopped claude card is left alone and a fresh
    // codex manager spawns — otherwise the setting would look applied and
    // silently keep reviving the old provider's manager.
    const hook = renderHook(() => useAgentManager());
    act(() =>
      hook.result.current.loadAgentsFromSession(
        [
          {
            id: 'agent-m',
            name: FLEET_MANAGER_NAME,
            cwd: '/home/u/Work',
            provider: 'claude',
            manager: true,
            lastSessionId: 'mgr-old',
            tabs: [
              {
                id: 't',
                title: 'M',
                panes: [{ id: 'p', type: 'claude', title: 'C' }],
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
      await hook.result.current.spawnFleetManager('status', '/home/u/Work', false, false, 'codex');
    });
    expect(spawnClaude).toHaveBeenCalledTimes(1);
    const opts = spawnClaude.mock.calls[0][0];
    expect(opts.provider).toBe('codex');
    // A FRESH manager, not a resume of the claude conversation.
    expect(opts.resumeSessionId).toBeUndefined();
    hook.unmount();
  });

  it('reuses a LIVE manager by name and discovers current workflow policy without a second spawn', async () => {
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
    await waitFor(() =>
      expect(claudeMessage).toHaveBeenCalledWith(
        'mgr-live',
        expect.stringContaining('and now?\n\nResolve inbox requests to pin new tasks'),
      ),
    );
    hook.unmount();
  });
});

it('prepares a modern manager bootstrap request before sending the first user ask', async () => {
  const prepare = vi
    .fn()
    .mockResolvedValue({ available: true, requestId: 'bootstrap-request', delivery: 'pending' });
  window.electronAPI.managerRequestPrepare = prepare;
  const spawn = window.electronAPI.spawnClaude as Mock;
  const message = window.electronAPI.claudeMessage as Mock;
  spawn.mockReset().mockResolvedValue('inbox-manager');
  message
    .mockReset()
    .mockResolvedValue({ ok: true, requestId: 'bootstrap-request', delivery: 'accepted' });
  const hook = renderHook(() => useAgentManager());
  try {
    await act(async () => {
      await hook.result.current.spawnFleetManager('Fix two things', '/project');
    });
    expect(spawn.mock.calls[0][0].message).toBeUndefined();
    expect(prepare).toHaveBeenCalledWith('inbox-manager', 'Fix two things', true);
    expect(prepare.mock.invocationCallOrder[0]).toBeLessThan(message.mock.invocationCallOrder[0]);
    expect(message).toHaveBeenCalledWith(
      'inbox-manager',
      buildManagerKickoff('Fix two things'),
      'bootstrap-request',
    );
  } finally {
    delete window.electronAPI.managerRequestPrepare;
    hook.unmount();
  }
});

it('opens a new manager without a task, then focuses it without changing its workspace', async () => {
  const prepare = vi.fn();
  window.electronAPI.managerRequestPrepare = prepare;
  const spawn = window.electronAPI.spawnClaude as Mock;
  const message = window.electronAPI.claudeMessage as Mock;
  spawn.mockReset().mockResolvedValue('empty-manager');
  message.mockReset();
  const hook = renderHook(() => useAgentManager());
  try {
    await act(async () => {
      await hook.result.current.spawnFleetManager('', '/original', false, true, 'codex');
    });
    const before = hook.result.current.agents.find((a) => a.sessionId === 'empty-manager');
    expect(before).toMatchObject({ cwd: '/original', manager: true, provider: 'codex' });
    expect(spawn.mock.calls[0][0].message).toBeUndefined();
    await act(async () => {
      await hook.result.current.spawnFleetManager('', '/different', true, false, 'claude');
    });
    expect(spawn).toHaveBeenCalledTimes(1);
    expect(message).not.toHaveBeenCalled();
    expect(prepare).not.toHaveBeenCalled();
    expect(hook.result.current.agents.find((a) => a.sessionId === 'empty-manager')).toBe(before);
  } finally {
    delete window.electronAPI.managerRequestPrepare;
  }
});

it('shares an in-flight guard across empty opening and a concurrent ask', async () => {
  let finish!: (id: string) => void;
  const spawn = window.electronAPI.spawnClaude as Mock;
  spawn.mockReset().mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const hook = renderHook(() => useAgentManager());
  await act(async () => {
    const opening = hook.result.current.spawnFleetManager('', '/project');
    await expect(
      hook.result.current.spawnFleetManager('keep my draft', '/project'),
    ).rejects.toThrow('already opening');
    finish('one-manager');
    await opening;
  });
  expect(spawn).toHaveBeenCalledTimes(1);
});

it('keeps Overview selected when request delivery fails and retries the captured identity', async () => {
  const spawn = window.electronAPI.spawnClaude as Mock;
  const message = window.electronAPI.claudeMessage as Mock;
  spawn.mockReset().mockResolvedValue('retry-manager');
  message.mockReset().mockResolvedValueOnce({ ok: false }).mockResolvedValue({ ok: true });
  const prepare = vi.fn(async () => ({
    available: true as const,
    requestId: 'stable-request',
    delivery: 'pending' as const,
  }));
  window.electronAPI.managerRequestPrepare = prepare;
  const hook = renderHook(() => useAgentManager());
  const initialSelection = hook.result.current.activeAgentId;
  try {
    await act(async () => {
      await expect(hook.result.current.spawnFleetManager('exact task', '/project')).rejects.toThrow(
        'could not start',
      );
    });
    expect(hook.result.current.activeAgentId).toBe(initialSelection);
    await act(async () => {
      await hook.result.current.spawnFleetManager('exact task', '/project');
    });
    expect(spawn).toHaveBeenCalledTimes(1);
    expect(prepare).toHaveBeenCalledTimes(1);
    expect(message.mock.calls.map((call) => call[2])).toEqual(['stable-request', 'stable-request']);
  } finally {
    delete window.electronAPI.managerRequestPrepare;
  }
});
