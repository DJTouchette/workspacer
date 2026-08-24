import { describe, it, expect } from 'vitest';

import {
  applyHookEvent,
  applyStopEvent,
  normalizeBackgroundAmbient,
  pendingSlotOwner,
  sessionHasBackgroundWork,
} from './hookEventRouter';
import type { ClaudeSessionState } from '../claudeSessionStore';

function mkSession(transport?: 'pty' | 'stream'): ClaudeSessionState {
  // Only the fields applyHookEvent touches matter here.
  return {
    sessionId: 's1',
    transport,
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    subagents: [],
    workflows: [],
    pendingApproval: null,
    pendingQuestions: null,
    ambientState: 'idle',
    totalToolCalls: 0,
  } as unknown as ClaudeSessionState;
}

describe('applyHookEvent — PTY sessions own their ambient state (unchanged behaviour)', () => {
  it('UserPromptSubmit flips a PTY session to streaming', () => {
    const s = mkSession();
    applyHookEvent(s, { hook_event_name: 'UserPromptSubmit' });
    expect(s.ambientState).toBe('streaming');
  });

  it('PermissionRequest parks the approval and flips to waiting_approval', () => {
    const s = mkSession('pty');
    applyHookEvent(s, {
      hook_event_name: 'PermissionRequest',
      tool_name: 'Bash',
      tool_input: { command: 'ls' },
    });
    expect(s.ambientState).toBe('waiting_approval');
    expect(s.pendingApproval?.toolName).toBe('Bash');
  });
});

describe('applyHookEvent — stream sessions: hooks are enrichment-only', () => {
  // The 'stream' transport (headless stream-json managed adapter) gets its
  // working/idle/waiting state from the daemon's managed mode stream
  // (set_managed_mode → applyManagedMode) — hooks must not write ambientState
  // or the two state machines fight. Everything else still applies.

  it('never writes ambientState from hooks', () => {
    const s = mkSession('stream');
    s.ambientState = 'thinking'; // as set by the managed mode stream
    for (const event of [
      { hook_event_name: 'SessionStart' },
      { hook_event_name: 'UserPromptSubmit' },
      { hook_event_name: 'PreToolUse', tool_use_id: 't1', tool_name: 'Bash', tool_input: {} },
      { hook_event_name: 'PostToolUse', tool_use_id: 't1' },
      { hook_event_name: 'PermissionRequest', tool_name: 'Bash', tool_input: {} },
    ]) {
      applyHookEvent(s, event);
      expect(s.ambientState).toBe('thinking');
    }
  });

  it('still enriches: tool cards, file changes, approval payloads', () => {
    const s = mkSession('stream');
    applyHookEvent(s, {
      hook_event_name: 'PreToolUse',
      tool_use_id: 't1',
      tool_name: 'Write',
      tool_input: { file_path: '/tmp/x' },
    });
    expect(s.activeToolCalls).toHaveLength(1);
    expect(s.fileChanges).toHaveLength(1);

    applyHookEvent(s, { hook_event_name: 'PostToolUse', tool_use_id: 't1' });
    expect(s.activeToolCalls).toHaveLength(0);
    expect(s.completedToolCalls).toHaveLength(1);

    // …but NOT the pending slot: on a stream session the daemon owns the
    // approval/question cards (see the ownership describe below).
    applyHookEvent(s, {
      hook_event_name: 'PermissionRequest',
      tool_name: 'Bash',
      tool_input: { command: 'rm -rf /' },
    });
    expect(s.pendingApproval).toBeNull();
  });

  it('leaves the AskUserQuestion picker to the daemon (it parks Pending::Question itself)', () => {
    const s = mkSession('stream');
    s.ambientState = 'streaming';
    applyHookEvent(s, {
      hook_event_name: 'PreToolUse',
      tool_use_id: 't2',
      tool_name: 'AskUserQuestion',
      tool_input: { questions: [{ question: 'Which?', options: [{ label: 'a' }] }] },
    });
    // claude_stream.rs parks AskUserQuestion as Pending::Question on the
    // /events feed; the tool card is still enrichment we keep.
    expect(s.pendingQuestions).toBeNull();
    expect(s.activeToolCalls).toHaveLength(1);
    expect(s.ambientState).toBe('streaming'); // mode comes from the daemon
  });

  it("a subagent's tool call (agent_id present) stays out of the live work log", () => {
    const s = mkSession('pty');
    s.pendingApproval = { toolName: 'Bash', toolInput: {}, timestamp: 1 } as never;
    applyHookEvent(s, {
      hook_event_name: 'PreToolUse',
      tool_use_id: 'sub-1',
      tool_name: 'Bash',
      tool_input: { command: 'echo hi' },
      agent_id: 'agent-abc',
      agent_type: 'general-purpose',
    });
    expect(s.activeToolCalls).toHaveLength(0);
    // …and it must not clear the parent's pending approval card.
    expect(s.pendingApproval).not.toBeNull();
    applyHookEvent(s, {
      hook_event_name: 'PostToolUse',
      tool_use_id: 'sub-1',
      tool_name: 'Bash',
      agent_id: 'agent-abc',
    });
    expect(s.pendingApproval).not.toBeNull();
  });

  it("a subagent's file edit is still recorded as a file change", () => {
    const s = mkSession('pty');
    applyHookEvent(s, {
      hook_event_name: 'PreToolUse',
      tool_use_id: 'sub-2',
      tool_name: 'Write',
      tool_input: { file_path: '/tmp/x.txt', content: 'hi' },
      agent_id: 'agent-abc',
    });
    expect(s.activeToolCalls).toHaveLength(0);
    expect(s.fileChanges).toHaveLength(1);
    expect(s.fileChanges[0].path).toBe('/tmp/x.txt');
  });

  it("normalizes the tool input's camelCase multiSelect to multi_select", () => {
    const s = mkSession('pty');
    applyHookEvent(s, {
      hook_event_name: 'PreToolUse',
      tool_use_id: 't3',
      tool_name: 'AskUserQuestion',
      tool_input: {
        questions: [
          { question: 'Pick several', multiSelect: true, options: [{ label: 'a' }] },
          { question: 'Pick one', options: [{ label: 'b' }] },
        ],
      },
    });
    expect(s.pendingQuestions?.[0].multi_select).toBe(true);
    expect(s.pendingQuestions?.[1].multi_select).toBe(false);
  });

  it('SessionStart still marks the session active', () => {
    const s = mkSession('stream');
    s.status = 'starting';
    applyHookEvent(s, { hook_event_name: 'SessionStart' });
    expect(s.status).toBe('active');
  });
});

describe('a late hook frame must not null an approval the daemon still holds', () => {
  // The unresolvable-approval bug (2026-08-22). A stream-transport session
  // learns about approvals over TWO independent SSE connections to claudemon
  // with no ordering guarantee between them: `/events` (in-process
  // set_managed_mode -> applyManagedMode) parks the card, `/hooks/stream` (a
  // `curl` subprocess out of the CLI's own hook) is the slower one. When a
  // hook frame landed second it nulled `pendingApproval` unconditionally while
  // `ambientState` stayed 'waiting_approval' (already guarded) — a session
  // visibly blocked with nothing to approve, unrecoverable short of killing it.

  function daemonParked(transport: 'pty' | 'stream'): ClaudeSessionState {
    const s = mkSession(transport);
    // What applyManagedPending writes when the daemon parks a can_use_tool.
    s.ambientState = 'waiting_approval';
    s.pendingApproval = { toolName: 'Read', toolInput: { file_path: '/x' }, timestamp: 1 } as never;
    return s;
  }

  const lateFrames = [
    [
      'PreToolUse for the next tool',
      { hook_event_name: 'PreToolUse', tool_use_id: 't9', tool_name: 'Bash', tool_input: {} },
    ],
    ['PostToolUse for the previous tool', { hook_event_name: 'PostToolUse', tool_use_id: 't8' }],
    [
      'an AskUserQuestion PreToolUse',
      {
        hook_event_name: 'PreToolUse',
        tool_use_id: 't7',
        tool_name: 'AskUserQuestion',
        tool_input: { questions: [{ question: 'Which?', options: [] }] },
      },
    ],
  ] as const;

  for (const [what, event] of lateFrames) {
    it(`stream: ${what} leaves the daemon's card alone`, () => {
      const s = daemonParked('stream');
      applyHookEvent(s, event);
      expect(s.pendingApproval).not.toBeNull();
      expect(s.pendingApproval?.toolName).toBe('Read');
      // The half that was already guarded, pinned alongside it: the pair is
      // what made the state unanswerable rather than merely stale.
      expect(s.ambientState).toBe('waiting_approval');
    });
  }

  it('stream: the whole observed shape — blocked state with a live card, not a null one', () => {
    const s = daemonParked('stream');
    for (const [, event] of lateFrames) applyHookEvent(s, event);
    expect(s.ambientState).toBe('waiting_approval');
    expect(s.pendingApproval).not.toBeNull();
  });

  it('a non-claude provider on the hook feed is daemon-owned too', () => {
    const s = daemonParked('pty');
    s.provider = 'codex';
    applyHookEvent(s, { hook_event_name: 'PostToolUse', tool_use_id: 't8' });
    expect(s.pendingApproval).not.toBeNull();
  });

  it('PTY claude still owns its own card: hooks set it AND clear it', () => {
    // The hook feed is the ONLY source of approvals for a PTY claude session
    // (PermissionRequest -> SessionMode::Approval in claudemon's state.rs), so
    // the guard must NOT make its cards unclearable.
    const s = mkSession('pty');
    applyHookEvent(s, {
      hook_event_name: 'PermissionRequest',
      tool_name: 'Read',
      tool_input: { file_path: '/x' },
    });
    expect(s.pendingApproval?.toolName).toBe('Read');
    expect(s.ambientState).toBe('waiting_approval');

    applyHookEvent(s, { hook_event_name: 'PostToolUse', tool_use_id: 't8' });
    expect(s.pendingApproval).toBeNull();
  });

  it('a session with no transport recorded yet is treated as PTY (hook-owned)', () => {
    const s = daemonParked('pty');
    delete (s as { transport?: string }).transport;
    applyHookEvent(s, { hook_event_name: 'PostToolUse', tool_use_id: 't8' });
    expect(s.pendingApproval).toBeNull();
  });

  it('a turn boundary sweeps the card this feed owns — and only that one', () => {
    // "The turn is over, so nothing can still be parked" holds within ONE
    // feed. For a stream session the owner's turn end is the driver's
    // `result` frame (AgentUpdate::Idle -> PendingWrite::Resolve), and this
    // Stop hook is the slow second feed: a background subagent's can_use_tool
    // parked before it lands would be stranded exactly as a late PostToolUse
    // stranded one.
    const pty = daemonParked('pty');
    applyStopEvent(pty);
    expect(pty.pendingApproval).toBeNull();

    const stream = daemonParked('stream');
    applyStopEvent(stream);
    expect(stream.pendingApproval).not.toBeNull();
    expect(stream.pendingApproval?.toolName).toBe('Read');
  });
});

describe('background subagents keep the pane busy past the parent Stop (PTY)', () => {
  const startSub = (s: ClaudeSessionState, id = 'agent-1') =>
    applyHookEvent(s, {
      hook_event_name: 'SubagentStart',
      agent_id: id,
      agent_type: 'general-purpose',
    });
  const stopSub = (s: ClaudeSessionState, id = 'agent-1') =>
    applyHookEvent(s, { hook_event_name: 'SubagentStop', agent_id: id });

  it('holds the background state when Stop fires while a subagent is still running', () => {
    const s = mkSession('pty');
    startSub(s);
    applyStopEvent(s);
    expect(s.ambientState).toBe('background');
    expect(s.parentTurnEnded).toBe(true);
    // The running subagent survives the Stop's tool-call cleanup.
    expect(s.subagents.some((sub) => sub.status === 'running')).toBe(true);
  });

  it('rides the real idle in on the last SubagentStop after the parent ended', () => {
    const s = mkSession('pty');
    startSub(s);
    applyStopEvent(s);
    stopSub(s);
    expect(s.ambientState).toBe('idle');
    expect(s.parentTurnEnded).toBe(false);
  });

  it('idles immediately at Stop when no subagent is running', () => {
    const s = mkSession('pty');
    s.ambientState = 'streaming';
    applyStopEvent(s);
    expect(s.ambientState).toBe('idle');
    expect(s.parentTurnEnded).toBeFalsy();
  });

  it('a subagent finishing mid-turn (before Stop) does not idle the parent', () => {
    const s = mkSession('pty');
    s.ambientState = 'streaming';
    startSub(s);
    stopSub(s);
    // Parent turn never ended, so ambient is untouched by SubagentStop.
    expect(s.ambientState).toBe('streaming');
    expect(s.parentTurnEnded).toBeFalsy();
  });

  it('waits for all parallel subagents to drain before idling', () => {
    const s = mkSession('pty');
    startSub(s, 'agent-1');
    startSub(s, 'agent-2');
    applyStopEvent(s);
    expect(s.ambientState).toBe('background');
    stopSub(s, 'agent-1');
    expect(s.ambientState).toBe('background'); // one still running
    stopSub(s, 'agent-2');
    expect(s.ambientState).toBe('idle');
  });

  it('a new UserPromptSubmit clears a stuck parentTurnEnded flag', () => {
    const s = mkSession('pty');
    startSub(s);
    applyStopEvent(s);
    expect(s.parentTurnEnded).toBe(true);
    applyHookEvent(s, { hook_event_name: 'UserPromptSubmit' });
    expect(s.parentTurnEnded).toBe(false);
    expect(s.ambientState).toBe('streaming');
  });

  it('stream sessions: Stop never touches ambient (daemon owns it, holds busy mid-subagent)', () => {
    const s = mkSession('stream');
    // The daemon's managed mode set this while the background subagent runs.
    s.ambientState = 'streaming';
    startSub(s);
    applyHookEvent(s, { hook_event_name: 'PreToolUse', tool_use_id: 't1', tool_name: 'Bash' });
    applyStopEvent(s);
    // Must NOT clobber the daemon-driven busy state back to idle.
    expect(s.ambientState).toBe('streaming');
    expect(s.parentTurnEnded).toBeFalsy();
    // The cleanup Stop DOES own on a stream session: the inline tool rows.
    // (`pendingApproval` used to be asserted null here — trivially true on a
    // session that never had a card, and since the slot became ownership-gated
    // it asserted the opposite of the invariant. What Stop must not do to a
    // daemon-parked card is pinned in 'turn boundary: Stop sweeps only what
    // this feed owns' below.)
    expect(s.activeToolCalls).toHaveLength(0);
    expect(s.completedToolCalls).toHaveLength(0);
  });
});

describe('SubagentStart idempotency (re-delivered hook)', () => {
  const startSub = (s: ClaudeSessionState, id = 'agent-abc') =>
    applyHookEvent(s, {
      hook_event_name: 'SubagentStart',
      agent_id: id,
      agent_type: 'general-purpose',
    });

  it('does not create a duplicate subagent when SubagentStart is re-delivered', () => {
    const s = mkSession('pty');
    startSub(s);
    startSub(s); // re-delivered (double-fired / retried hook POST)
    expect(s.subagents).toHaveLength(1);
  });

  it('a re-delivered SubagentStart leaves no phantom running subagent after Stop', () => {
    const s = mkSession('pty');
    startSub(s);
    startSub(s); // duplicate
    applyStopEvent(s); // parent turn ends while the subagent runs
    expect(s.ambientState).toBe('background');
    // The single SubagentStop must drain the subagent for that id, not leave a
    // second stuck 'running' entry pinning the parent on 'background' forever.
    applyHookEvent(s, { hook_event_name: 'SubagentStop', agent_id: 'agent-abc' });
    expect(s.subagents.some((sub) => sub.status === 'running')).toBe(false);
    expect(sessionHasBackgroundWork(s)).toBe(false);
    expect(s.ambientState).toBe('idle');
  });
});

describe('applyHookEvent — PostToolUse completion + failure', () => {
  it('marks a tool complete on a clean PostToolUse', () => {
    const s = mkSession('pty');
    applyHookEvent(s, { hook_event_name: 'PreToolUse', tool_use_id: 'tu1', tool_name: 'Bash' });
    applyHookEvent(s, {
      hook_event_name: 'PostToolUse',
      tool_use_id: 'tu1',
      tool_response: { stdout: 'ok' },
    });
    expect(s.activeToolCalls).toHaveLength(0);
    expect(s.completedToolCalls[0]?.status).toBe('complete');
    expect(s.completedToolCalls[0]?.completedAt).toBeDefined();
  });

  it('marks a tool failed when tool_response is an error', () => {
    const s = mkSession('pty');
    applyHookEvent(s, { hook_event_name: 'PreToolUse', tool_use_id: 'tu2', tool_name: 'Bash' });
    applyHookEvent(s, {
      hook_event_name: 'PostToolUse',
      tool_use_id: 'tu2',
      tool_response: { is_error: true, stderr: 'boom' },
    });
    expect(s.completedToolCalls[0]?.status).toBe('failed');
  });
});

describe('normalizeBackgroundAmbient — workflows keep idle honest', () => {
  const runningWf = { runId: 'wf1', status: 'running', startedAt: 1, phases: [] };
  const doneWf = { runId: 'wf1', status: 'completed', startedAt: 1, phases: [] };

  it('an idle session with a running workflow reads background', () => {
    const s = mkSession('pty');
    s.workflows = [runningWf] as ClaudeSessionState['workflows'];
    normalizeBackgroundAmbient(s);
    expect(s.ambientState).toBe('background');
  });

  it('drops back to idle when the workflow finishes', () => {
    const s = mkSession('pty');
    s.ambientState = 'background';
    s.workflows = [doneWf] as ClaudeSessionState['workflows'];
    normalizeBackgroundAmbient(s);
    expect(s.ambientState).toBe('idle');
  });

  it('never rewrites active states (streaming / waiting) or ended sessions', () => {
    const s = mkSession('pty');
    s.workflows = [runningWf] as ClaudeSessionState['workflows'];
    for (const state of ['streaming', 'thinking', 'waiting_approval', 'waiting_input'] as const) {
      s.ambientState = state;
      normalizeBackgroundAmbient(s);
      expect(s.ambientState).toBe(state);
    }
    const ended = mkSession('pty');
    ended.status = 'ended';
    ended.workflows = [runningWf] as ClaudeSessionState['workflows'];
    normalizeBackgroundAmbient(ended);
    expect(ended.ambientState).toBe('idle');
  });

  it('Stop with a running workflow lands on background via normalize (the PTY workflow case)', () => {
    const s = mkSession('pty');
    s.ambientState = 'streaming';
    s.workflows = [runningWf] as ClaudeSessionState['workflows'];
    applyStopEvent(s); // no subagents → Stop itself writes idle
    expect(s.ambientState).toBe('idle');
    normalizeBackgroundAmbient(s); // …but the store normalizes before notifying
    expect(s.ambientState).toBe('background');
  });

  it('sessionHasBackgroundWork sees running workflows and running subagents only', () => {
    const s = mkSession('pty');
    expect(sessionHasBackgroundWork(s)).toBe(false);
    s.workflows = [doneWf] as ClaudeSessionState['workflows'];
    expect(sessionHasBackgroundWork(s)).toBe(false);
    s.workflows = [runningWf] as ClaudeSessionState['workflows'];
    expect(sessionHasBackgroundWork(s)).toBe(true);
    s.workflows = [];
    s.subagents = [{ id: 'a1', type: 't', status: 'running', startedAt: 1 }];
    expect(sessionHasBackgroundWork(s)).toBe(true);
    s.subagents[0].status = 'complete';
    expect(sessionHasBackgroundWork(s)).toBe(false);
  });
});

describe('the pending slot has exactly one owner', () => {
  // INVARIANT: exactly one feed owns a session's pending slot; others may
  // enrich but never park or resolve it. The hook feed owns it for claude on
  // the PTY transport only — for a stream session or any non-claude provider
  // the daemon's `pending` slot (applyManagedPending) is the sole writer, and
  // this feed is the laggy one racing it over a separate SSE connection.

  function daemonQuestion(transport: 'pty' | 'stream'): ClaudeSessionState {
    const s = mkSession(transport);
    s.ambientState = 'waiting_input';
    s.pendingQuestions = [
      { question: 'Which?', options: [{ label: 'a' }], multi_select: false },
    ] as never;
    return s;
  }

  it('names the owner from ./pendingSlot — one function, so the feeds cannot disagree', () => {
    expect(pendingSlotOwner({ provider: 'claude', transport: 'pty' })).toBe('hooks');
    expect(pendingSlotOwner({} as ClaudeSessionState)).toBe('hooks'); // default claude/pty
    expect(pendingSlotOwner({ provider: 'claude', transport: 'stream' })).toBe('daemon');
    expect(pendingSlotOwner({ provider: 'codex', transport: 'pty' })).toBe('daemon');
    expect(pendingSlotOwner({ provider: 'pi' } as ClaudeSessionState)).toBe('daemon');
  });

  describe("a row mirrored from a peer hub is not this feed's either", () => {
    // It looks exactly like a session the hook feed owns — claude, PTY — but
    // the request is parked on the OTHER machine. Ownership read off
    // provider/transport alone said 'hooks' and let hooks write the mirror;
    // an Approve rendered from that card posts a decision the local daemon has
    // never heard of, and the peer's request stays open.
    const remote = (): ClaudeSessionState => {
      const s = mkSession('pty');
      s.provider = 'claude';
      s.hub = 'laptop';
      return s;
    };

    it("a PermissionRequest cannot park a card on the peer's row", () => {
      const s = remote();
      applyHookEvent(s, {
        hook_event_name: 'PermissionRequest',
        tool_name: 'Bash',
        tool_input: { command: 'ls' },
      });
      expect(s.pendingApproval).toBeNull();
    });

    it("an AskUserQuestion PreToolUse cannot park the peer's picker", () => {
      const s = remote();
      applyHookEvent(s, {
        hook_event_name: 'PreToolUse',
        tool_use_id: 'q1',
        tool_name: 'AskUserQuestion',
        tool_input: { questions: [{ question: 'Which?', options: [] }] },
      });
      expect(s.pendingQuestions).toBeNull();
    });

    it('Stop cannot sweep what the peer is still holding open', () => {
      const s = remote();
      s.pendingApproval = {
        toolName: 'Read',
        toolInput: { file_path: '/peer/x' },
        timestamp: 7,
      } as never;
      s.pendingQuestions = [{ question: 'Which?', options: [] }] as never;
      applyStopEvent(s);
      expect(s.pendingApproval?.toolName).toBe('Read');
      expect(s.pendingQuestions).toHaveLength(1);
    });
  });

  describe('questions: the hook feed neither parks nor resolves a daemon-owned picker', () => {
    it('stream: a late AskUserQuestion PostToolUse does not answer-by-forgetting', () => {
      // The strand: the driver still holds the AskUserQuestion `can_use_tool`
      // open, ambientState stays 'waiting_input' (already guarded), and the
      // picker the user must answer through is gone.
      const s = daemonQuestion('stream');
      applyHookEvent(s, {
        hook_event_name: 'PreToolUse',
        tool_use_id: 'q1',
        tool_name: 'AskUserQuestion',
        tool_input: { questions: [{ question: 'Which?', options: [{ label: 'a' }] }] },
      });
      applyHookEvent(s, { hook_event_name: 'PostToolUse', tool_use_id: 'q1' });
      expect(s.pendingQuestions).not.toBeNull();
      expect(s.pendingQuestions).toHaveLength(1);
      expect(s.ambientState).toBe('waiting_input');
    });

    it('codex: same, on a non-claude provider that fires no hooks of its own', () => {
      const s = daemonQuestion('pty');
      s.provider = 'codex';
      applyHookEvent(s, {
        hook_event_name: 'PreToolUse',
        tool_use_id: 'q1',
        tool_name: 'AskUserQuestion',
        tool_input: { questions: [{ question: 'Injected?', options: [] }] },
      });
      expect(s.pendingQuestions?.[0].question).toBe('Which?');
      applyHookEvent(s, { hook_event_name: 'PostToolUse', tool_use_id: 'q1' });
      expect(s.pendingQuestions).not.toBeNull();
    });

    it('PTY claude still owns the picker end to end: parked, then cleared', () => {
      const s = mkSession('pty');
      applyHookEvent(s, {
        hook_event_name: 'PreToolUse',
        tool_use_id: 'q1',
        tool_name: 'AskUserQuestion',
        tool_input: { questions: [{ question: 'Which?', options: [{ label: 'a' }] }] },
      });
      expect(s.pendingQuestions).toHaveLength(1);
      expect(s.ambientState).toBe('waiting_input');
      applyHookEvent(s, { hook_event_name: 'PostToolUse', tool_use_id: 'q1' });
      expect(s.pendingQuestions).toBeNull();
    });
  });

  describe('approvals: PermissionRequest can add a card too, and that is not harmless', () => {
    it('stream: does not resurrect an approval the daemon already resolved', () => {
      // The hook is a curl subprocess; its PermissionRequest can land after
      // the driver answered and cleared the request. Writing the card then
      // shows an Approve/Deny the daemon will reject as unknown.
      const s = mkSession('stream');
      s.ambientState = 'streaming'; // daemon already resumed the turn
      applyHookEvent(s, {
        hook_event_name: 'PermissionRequest',
        tool_name: 'Bash',
        tool_input: { command: 'ls' },
      });
      expect(s.pendingApproval).toBeNull();
    });

    it("stream: does not surface a queued approval over the daemon's head card", () => {
      // claude_stream.rs parks ONE can_use_tool at a time and re-surfaces the
      // next only when the head is answered; the hooks fire for all of them.
      const s = mkSession('stream');
      s.ambientState = 'waiting_approval';
      s.pendingApproval = {
        toolName: 'Read',
        toolInput: { file_path: '/x' },
        timestamp: 1,
      } as never;
      applyHookEvent(s, {
        hook_event_name: 'PermissionRequest',
        tool_name: 'Bash',
        tool_input: { command: 'rm -rf /' },
      });
      expect(s.pendingApproval?.toolName).toBe('Read');
    });

    it('codex: a hook-shaped PermissionRequest cannot write a daemon-owned card', () => {
      const s = mkSession('pty');
      s.provider = 'codex';
      applyHookEvent(s, {
        hook_event_name: 'PermissionRequest',
        tool_name: 'Bash',
        tool_input: {},
      });
      expect(s.pendingApproval).toBeNull();
    });
  });

  describe('turn boundary: Stop sweeps only what this feed owns', () => {
    it('stream: a Stop landing after the driver parked a background request leaves it', () => {
      const s = mkSession('stream');
      s.ambientState = 'waiting_approval';
      s.pendingApproval = { toolName: 'Bash', toolInput: {}, timestamp: 1 } as never;
      s.pendingQuestions = [{ question: 'Which?', options: [] }] as never;
      applyStopEvent(s);
      expect(s.pendingApproval).not.toBeNull();
      expect(s.pendingQuestions).not.toBeNull();
      // Everything Stop legitimately owns still happens.
      expect(s.activeToolCalls).toHaveLength(0);
      expect(s.ambientState).toBe('waiting_approval');
    });

    it('PTY claude: Stop still sweeps both halves of its own slot', () => {
      const s = mkSession('pty');
      s.pendingApproval = { toolName: 'Bash', toolInput: {}, timestamp: 1 } as never;
      s.pendingQuestions = [{ question: 'Which?', options: [] }] as never;
      applyStopEvent(s);
      expect(s.pendingApproval).toBeNull();
      expect(s.pendingQuestions).toBeNull();
    });
  });
});
