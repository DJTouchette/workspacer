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
    fileChanges: [],
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

  it('detects a trailing interrupt marker tagged with `type` instead of `kind`', () => {
    const s = mkSession();
    // The main switch resolves the discriminant as `kind ?? type`; the interrupt
    // path must be equally tolerant or a type-tagged marker leaves it streaming.
    applyConversationItems(
      s,
      [{ type: 'user_message', text: '[Request interrupted by user]' } as ConversationItemWire],
      noUsage,
    );
    expect(s.ambientState).toBe('idle');
    expect(s.pendingApproval).toBeNull();
  });

  it('detects a type-tagged interrupt even behind a trailing type-tagged usage item', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [
        {
          type: 'tool_result',
          tool_use_id: 'tu_1',
          content: '[Request interrupted by user for tool use]',
          is_error: true,
        } as ConversationItemWire,
        {
          type: 'usage',
          model: 'claude-sonnet-4-5',
          usage: {},
          message_id: 'm1',
        } as ConversationItemWire,
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

describe('applyConversationItems — plan', () => {
  it('a plan item sets session.plan (full replacement)', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [
        {
          kind: 'plan',
          steps: [
            { content: 'Write types', status: 'completed' },
            { content: 'Wire the UI', status: 'in_progress', activeForm: 'Wiring the UI' },
            { content: 'Add tests', status: 'pending' },
          ],
          updatedAt: 123,
        },
      ],
      noUsage,
    );
    expect(s.plan?.steps).toHaveLength(3);
    expect(s.plan?.steps[1]).toEqual({
      content: 'Wire the UI',
      status: 'in_progress',
      activeForm: 'Wiring the UI',
    });
    expect(s.plan?.updatedAt).toBe(123);
  });

  it('tolerates the `type` discriminant and snake_case active_form / updated_at', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [
        {
          type: 'plan',
          steps: [{ content: 'Do it', status: 'in_progress', active_form: 'Doing it' }],
          updated_at: 42,
        } as never,
      ],
      noUsage,
    );
    expect(s.plan?.steps[0].activeForm).toBe('Doing it');
    expect(s.plan?.updatedAt).toBe(42);
  });

  it('a TodoWrite tool_use call sets session.plan as a fallback', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [
        {
          kind: 'tool_use',
          id: 'tu_todo',
          name: 'TodoWrite',
          input: {
            todos: [
              { content: 'Step one', status: 'completed', activeForm: 'Doing step one' },
              { content: 'Step two', status: 'pending', activeForm: 'Doing step two' },
            ],
          },
        },
      ],
      noUsage,
    );
    expect(s.plan?.steps).toHaveLength(2);
    expect(s.plan?.steps[0].status).toBe('completed');
  });

  it('a later plan item replaces an earlier one (last-write-wins)', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [{ kind: 'plan', steps: [{ content: 'Only step', status: 'pending' }], updatedAt: 1 }],
      noUsage,
    );
    applyConversationItems(
      s,
      [
        {
          kind: 'plan',
          steps: [
            { content: 'New A', status: 'completed' },
            { content: 'New B', status: 'in_progress' },
          ],
          updatedAt: 2,
        },
      ],
      noUsage,
    );
    expect(s.plan?.steps).toHaveLength(2);
    expect(s.plan?.steps[0].content).toBe('New A');
    expect(s.plan?.updatedAt).toBe(2);
  });

  it('drops empty-content rows and defaults unknown status to pending', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [
        {
          kind: 'plan',
          steps: [
            { content: '', status: 'completed' },
            { content: 'Real', status: 'bogus' as never },
          ],
        },
      ],
      noUsage,
    );
    expect(s.plan?.steps).toHaveLength(1);
    expect(s.plan?.steps[0]).toEqual({ content: 'Real', status: 'pending' });
  });
});

describe('applyConversationItems — assistant text coalescing by transport', () => {
  const deltas: ConversationItemWire[] = [
    { kind: 'assistant_text', text: 'Here' },
    { kind: 'assistant_text', text: ' is the' },
    { kind: 'assistant_text', text: ' plan:\n```ts\nconst x = 1;\n```' },
  ];

  it("claude 'stream' transport coalesces per-token deltas into one turn", () => {
    const s = mkSession();
    (s as { provider?: string }).provider = 'claude';
    (s as { transport?: string }).transport = 'stream';
    applyConversationItems(s, deltas, noUsage);
    expect(s.conversation).toHaveLength(1);
    expect(s.conversation[0].content).toBe('Here is the plan:\n```ts\nconst x = 1;\n```');
  });

  it('claude PTY transport keeps whole-block push semantics (one turn per block)', () => {
    const s = mkSession();
    (s as { provider?: string }).provider = 'claude';
    applyConversationItems(
      s,
      [
        { kind: 'assistant_text', text: 'Block one.' },
        { kind: 'assistant_text', text: 'Block two.' },
      ],
      noUsage,
    );
    expect(s.conversation).toHaveLength(2);
  });

  it('a stream delta after a tool call starts a fresh bubble instead of merging into it', () => {
    const s = mkSession();
    (s as { provider?: string }).provider = 'claude';
    (s as { transport?: string }).transport = 'stream';
    applyConversationItems(
      s,
      [
        { kind: 'assistant_text', text: 'Before.' },
        { kind: 'tool_use', id: 'tu_9', name: 'Bash', input: {} },
        { kind: 'assistant_text', text: 'After.' },
      ],
      noUsage,
    );
    expect(s.conversation).toHaveLength(3);
    expect(s.conversation[2].content).toBe('After.');
  });
});

describe('applyConversationItems — managed-provider file changes', () => {
  it('a codex apply_patch tool_use records a fileChange (managed providers fire no hooks)', () => {
    const s = mkSession();
    (s as { provider?: string }).provider = 'codex';
    applyConversationItems(
      s,
      [
        {
          kind: 'tool_use',
          id: 'tu_p1',
          name: 'apply_patch',
          input: { path: '/w/src/a.rs', diff: '@@\n-old\n+new' },
        },
      ],
      noUsage,
    );
    expect(s.fileChanges).toHaveLength(1);
    expect(s.fileChanges[0].path).toBe('/w/src/a.rs');
    expect(s.fileChanges[0].toolName).toBe('apply_patch');
  });

  it('a multi-file apply_patch (changes array) records one fileChange per path', () => {
    const s = mkSession();
    (s as { provider?: string }).provider = 'codex';
    applyConversationItems(
      s,
      [
        {
          kind: 'tool_use',
          id: 'tu_p2',
          name: 'apply_patch',
          input: {
            path: 'src/a.rs',
            changes: [
              { path: 'src/a.rs', kind: 'update', diff: '@@\n+x' },
              { path: 'src/b.rs', kind: 'add', diff: '@@\n+y' },
            ],
          },
        },
      ],
      noUsage,
    );
    expect(s.fileChanges.map((fc) => fc.path)).toEqual(['src/a.rs', 'src/b.rs']);
  });

  it('a re-delivered tool_use id does not double-record the fileChange', () => {
    const s = mkSession();
    (s as { provider?: string }).provider = 'codex';
    const call: ConversationItemWire = {
      kind: 'tool_use',
      id: 'tu_p3',
      name: 'apply_patch',
      input: { path: 'src/a.rs', diff: '@@\n+x' },
    };
    applyConversationItems(s, [call], noUsage);
    applyConversationItems(s, [{ ...call }], noUsage);
    expect(s.fileChanges).toHaveLength(1);
  });

  it('claude sessions keep the hook path — no fileChange recorded here', () => {
    const s = mkSession();
    (s as { provider?: string }).provider = 'claude';
    applyConversationItems(
      s,
      [
        {
          kind: 'tool_use',
          id: 'tu_p4',
          name: 'Edit',
          input: { file_path: '/w/x.ts', old_string: 'a', new_string: 'b' },
        },
      ],
      noUsage,
    );
    expect(s.fileChanges).toHaveLength(0);
  });

  it('non-edit managed tools (shell) record nothing', () => {
    const s = mkSession();
    (s as { provider?: string }).provider = 'codex';
    applyConversationItems(
      s,
      [{ kind: 'tool_use', id: 'tu_p5', name: 'shell', input: { command: 'ls' } }],
      noUsage,
    );
    expect(s.fileChanges).toHaveLength(0);
  });
});

describe('applyConversationItems — tool_use dedup', () => {
  const call: ConversationItemWire = {
    kind: 'tool_use',
    id: 'toolu_dup',
    name: 'Bash',
    input: { command: 'echo hi' },
  };

  it('drops a re-delivered tool_use in the same batch', () => {
    const s = mkSession();
    applyConversationItems(s, [call, { ...call }], noUsage);
    expect(s.conversation).toHaveLength(1);
    expect(s.totalToolCalls).toBe(1);
  });

  it('drops a re-delivered tool_use across batches (compaction/resume replay)', () => {
    const s = mkSession();
    applyConversationItems(s, [call], noUsage);
    applyConversationItems(
      s,
      [{ kind: 'assistant_text', text: 'still working' }, { ...call }],
      noUsage,
    );
    expect(s.conversation.filter((t) => t.toolCalls?.length)).toHaveLength(1);
  });

  it('distinct ids still land as distinct turns', () => {
    const s = mkSession();
    applyConversationItems(s, [call, { ...call, id: 'toolu_other' }], noUsage);
    expect(s.conversation).toHaveLength(2);
  });
});

describe('applyConversationItems — tool durations', () => {
  it('stamps completedAt from the tool_result timestamp so durations are not 0s', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [
        {
          kind: 'tool_use',
          id: 'toolu_dur',
          name: 'Bash',
          input: { command: 'sleep 5' },
          timestamp: '2026-07-09T10:00:00Z',
        },
        {
          kind: 'tool_result',
          tool_use_id: 'toolu_dur',
          content: 'done',
          timestamp: '2026-07-09T10:00:05Z',
        },
      ],
      noUsage,
    );
    const tc = s.conversation.find((t) => t.toolCalls?.length)?.toolCalls?.[0];
    expect(tc).toBeDefined();
    expect(tc!.startedAt).toBe(Date.parse('2026-07-09T10:00:00Z'));
    expect(tc!.completedAt).toBe(Date.parse('2026-07-09T10:00:05Z'));
  });

  it('leaves completedAt alone when the result carries no timestamp (resync must not inflate)', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [
        {
          kind: 'tool_use',
          id: 'toolu_nots',
          name: 'Bash',
          input: {},
          timestamp: '2026-07-09T10:00:00Z',
        },
        { kind: 'tool_result', tool_use_id: 'toolu_nots', content: 'done' },
      ],
      noUsage,
    );
    const tc = s.conversation.find((t) => t.toolCalls?.length)?.toolCalls?.[0];
    expect(tc!.completedAt).toBe(tc!.startedAt);
  });

  it('ignores a result timestamp earlier than the call (clock skew reads as 0s, not negative)', () => {
    const s = mkSession();
    applyConversationItems(
      s,
      [
        {
          kind: 'tool_use',
          id: 'toolu_skew',
          name: 'Bash',
          input: {},
          timestamp: '2026-07-09T10:00:10Z',
        },
        {
          kind: 'tool_result',
          tool_use_id: 'toolu_skew',
          content: 'done',
          timestamp: '2026-07-09T10:00:05Z',
        },
      ],
      noUsage,
    );
    const tc = s.conversation.find((t) => t.toolCalls?.length)?.toolCalls?.[0];
    expect(tc!.completedAt).toBe(tc!.startedAt);
  });
});
