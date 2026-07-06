import { describe, it, expect } from 'vitest';

import { applyHookEvent } from './hookEventRouter';
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
