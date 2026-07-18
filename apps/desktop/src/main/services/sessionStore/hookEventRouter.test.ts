import { describe, it, expect } from 'vitest';

import {
  applyHookEvent,
  applyStopEvent,
  normalizeBackgroundAmbient,
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

    applyHookEvent(s, {
      hook_event_name: 'PermissionRequest',
      tool_name: 'Bash',
      tool_input: { command: 'rm -rf /' },
    });
    expect(s.pendingApproval?.toolName).toBe('Bash');
  });

  it('still surfaces AskUserQuestion pickers (payload without the mode flip)', () => {
    const s = mkSession('stream');
    s.ambientState = 'streaming';
    applyHookEvent(s, {
      hook_event_name: 'PreToolUse',
      tool_use_id: 't2',
      tool_name: 'AskUserQuestion',
      tool_input: { questions: [{ question: 'Which?', options: [{ label: 'a' }] }] },
    });
    expect(s.pendingQuestions).toHaveLength(1);
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
    applyStopEvent(s);
    // Must NOT clobber the daemon-driven busy state back to idle.
    expect(s.ambientState).toBe('streaming');
    expect(s.parentTurnEnded).toBeFalsy();
    // Non-ambient cleanup still applies.
    expect(s.pendingApproval).toBeNull();
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
