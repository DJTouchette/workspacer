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

describe('buildManagerKickoff — crash succession', () => {
  // Version rot, not logic: the doctrine taught the manager to find a dead
  // predecessor by scanning list_agents for a parentSessionId with no session
  // row of its own. That was the only method available until `list_orphans`
  // shipped (600a2c4e), and it is strictly worse than the tool — it cannot see
  // an EVICTED manager at all (the store keeps a tombstone the read has, the
  // agent list does not), and it cannot tell a dead manager from a worker that
  // spawned agents of its own. Teaching the old method now sends a successor
  // down a path that silently mis-adopts.
  const doctrine = buildManagerKickoff('go');

  it('names list_orphans as the answer when a predecessor crashed', () => {
    expect(doctrine).toContain('list_orphans');
    expect(doctrine).toMatch(/crashed[\s\S]{0,120}list_orphans|list_orphans[\s\S]{0,200}crash/i);
  });

  it('no longer teaches deriving a dead manager from list_agents', () => {
    expect(doctrine).not.toMatch(/list_agents still tells you/);
    expect(doctrine).not.toMatch(/parentSessionId that has no session row/);
  });

  it('warns that a candidate is a lead, not an answer', () => {
    // list_orphans reports and never picks; adopting the wrong group re-points
    // another manager's workers with nothing saying so.
    expect(doctrine).toContain('confirmedManager:false');
    expect(doctrine).toMatch(/adopt_workers’? fromSessionId/);
  });
});

describe('buildManagerKickoff — reviewer independence', () => {
  // Invariant 3 of the routing spec, landed as doctrine rather than as routing
  // machinery: review must not inherit the implementer's reasoning. The habit
  // only exists here, so each half of it is pinned — a separate worker, what
  // the reviewer is given, what it is deliberately NOT given, and the model
  // guidance that keeps a reviewer-per-ship-task affordable.
  const doctrine = buildManagerKickoff('go');

  it('names REVIEW as a task shape alongside ship and scout', () => {
    expect(doctrine).toMatch(/SHIP task, a SCOUT task, or a REVIEW task/);
    expect(doctrine).toMatch(/A REVIEW task follows every ship task that lands/);
  });

  it('forbids asking the implementer to grade its own work', () => {
    expect(doctrine).toMatch(/Never ask the implementer whether its own work is right/);
    expect(doctrine).toMatch(/reasoning that wrote the code cannot grade it/);
  });

  it('says a fresh session makes independence the default, and naming the one way to lose it', () => {
    expect(doctrine).toMatch(/spawn_agent always starts a FRESH session/);
    expect(doctrine).toMatch(/paste the implementer’s reasoning into the reviewer’s first message/);
  });

  it('lists what the reviewer is given, and what it is withheld', () => {
    // The spec's list: ticket, acceptance criteria, architectural constraints,
    // final diff, relevant files, test results.
    expect(doctrine).toMatch(/acceptance criteria/);
    expect(doctrine).toMatch(/architectural constraints/);
    expect(doctrine).toMatch(/branch or commit and its diff/);
    expect(doctrine).toMatch(/test results/);
    expect(doctrine).toMatch(
      /Do NOT give it the implementer’s plan, its reasoning, or its transcript/,
    );
  });

  it('prefers a different model family and keeps the reviewer tier cheap', () => {
    expect(doctrine).toMatch(/different model FAMILY/);
    expect(doctrine).toMatch(/list_models/);
    expect(doctrine).toMatch(/doubles your worker count/);
  });
});

describe('buildManagerKickoff — dispatch templates', () => {
  // The doctrine is the only place a manager learns templates exist; the text
  // is paid for in every manager session, so it is short — but the three
  // load-bearing pieces must stay: the kind and how to invoke it, the default
  // resultSchema behaviour, and the hard rule that the task slot is the
  // manager's own writing (an unfilled required placeholder REFUSES the spawn,
  // it never silently defaults).
  const doctrine = buildManagerKickoff('go');

  it('teaches the dispatch-template invocation shape on spawn_agent', () => {
    expect(doctrine).toContain('DISPATCH TEMPLATES');
    expect(doctrine).toContain('"template":"<item id>"');
    expect(doctrine).toContain('"templateParams"');
    // Discovery: templates are ordinary library items.
    expect(doctrine).toMatch(/kind "dispatch"[\s\S]{0,80}list_library/);
  });

  it('says the default resultSchema applies unless the call passes its own', () => {
    expect(doctrine).toMatch(/default resultSchema unless you pass your own/);
  });

  it('states the hard rule: unfilled required placeholder refuses the spawn, the task slot is the manager’s', () => {
    expect(doctrine).toMatch(/unfilled required placeholder REFUSES the spawn/i);
    expect(doctrine).toMatch(/task slot is yours to write/);
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
    // Point workers at the project's own code-intel tools when it has them.
    expect(kickoff).toContain('rivet');
    expect(kickoff).toContain('witness.select');
    // Workers report BY FINISHING — the manager must tell them so (a plain
    // worker has no channel back).
    expect(kickoff).toContain('end your turn with a short summary');
    expect(kickoff).toContain('delivered to me automatically');
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
    expect(kickoff).toContain('/handoff');
    // A successor manager must find a predecessor's handoff on its FIRST turn
    // without being told — the doctrine points at it beside the fleet brief.
    expect(kickoff).toContain('.workspacer/handoff.md');
    // Briefs prune by cold archival, not deletion; the fleet brief has a ## User
    // prefs section the manager honors.
    expect(kickoff).toContain('brief.archive.md');
    expect(kickoff).toContain('## User');
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
