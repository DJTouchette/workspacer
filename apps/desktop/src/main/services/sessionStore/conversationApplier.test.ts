import { describe, it, expect } from 'vitest';

import { applyConversationItems, type ConversationItemWire } from './conversationApplier';
import type { ClaudeSessionState } from '../claudeSessionStore';

const noUsage = () => {};

function mkSession(): ClaudeSessionState {
  // Only the fields applyConversationItems touches matter here.
  return {
    sessionId: 's1',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    subagents: [],
    pendingApproval: { toolName: 'Bash', toolInput: {}, timestamp: 1 },
    pendingQuestions: null,
    ambientState: 'streaming',
    totalToolCalls: 0,
  } as unknown as ClaudeSessionState;
}

const interruptedToolResult: ConversationItemWire = {
  kind: 'tool_result',
  tool_use_id: 'tu_1',
  content: '[Request interrupted by user for tool use]',
  is_error: true,
};

describe('applyConversationItems — interrupt detection', () => {
  it('a trailing interrupt marker ends the turn like Stop would (no Stop hook fires on interrupt)', () => {
    const s = mkSession();
    s.activeToolCalls.push({
      id: 'tu_1',
      name: 'Bash',
      input: {},
      status: 'running',
      startedAt: 1,
    });
    applyConversationItems(
      s,
      [{ kind: 'tool_use', id: 'tu_1', name: 'Bash', input: {} }, interruptedToolResult],
      noUsage,
    );
    expect(s.ambientState).toBe('idle');
    expect(s.pendingApproval).toBeNull();
    expect(s.activeToolCalls).toEqual([]);
  });

  it('a plain text interrupt (no tool pending) also ends the turn', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [{ kind: 'user_message', text: '[Request interrupted by user]' }],
      noUsage,
    );
    expect(s.ambientState).toBe('idle');
  });

  it('a mid-batch interrupt is history — later items win, state stays live', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [interruptedToolResult, { kind: 'user_message', text: 'ok now do this instead' }],
      noUsage,
    );
    expect(s.ambientState).toBe('streaming');
    expect(s.pendingApproval).not.toBeNull();
  });

  it('a trailing usage item does not mask an interrupt right before it', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [
        { kind: 'user_message', text: '[Request interrupted by user]' },
        { kind: 'usage', model: 'claude-sonnet-4-5', usage: {}, message_id: 'm1' },
      ],
      noUsage,
    );
    expect(s.ambientState).toBe('idle');
  });

  it('ordinary user messages do not trip the marker check', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [{ kind: 'user_message', text: 'please fix the [Request interrupted by user] handling' }],
      noUsage,
    );
    expect(s.ambientState).toBe('streaming');
  });
});
