/**
 * These bounds are load-bearing for main-process responsiveness, not just for
 * memory: every coalesced flush structured-clones the whole session to the
 * renderer, up to ~60 times a second while an agent streams. Anything that can
 * grow here becomes an unbounded per-frame cost on the thread that also
 * forwards PTY bytes, so a regression shows up as the whole app stuttering
 * rather than as anything resembling a memory bug.
 */
import { describe, expect, it } from 'vitest';
import {
  capConversationInPlace,
  capInPlace,
  MAX_CONVERSATION_TURNS,
  MAX_FILE_CHANGES,
  MAX_TOOL_INPUT_CHARS,
  MAX_TOOL_RESPONSE_CHARS,
  resetConversationOffsetIfRebuilt,
  truncateToolInput,
  truncateToolResponse,
} from './bounds';
import { applyConversationItems, type ConversationItemWire } from './conversationApplier';
import { applyHookEvent } from './hookEventRouter';
import type { ClaudeSessionState } from '../claudeSessionStore';

/** Only the fields these two ingest paths touch matter here. */
function mkSession(): ClaudeSessionState {
  return {
    sessionId: 's1',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    subagents: [],
    workflows: [],
    pendingApproval: null,
    pendingQuestions: null,
    ambientState: 'streaming',
    totalToolCalls: 0,
  } as unknown as ClaudeSessionState;
}

/** The absolute index of a turn is offset + array index; every consumer that
 *  anchors by index (ClaudePane rows, turn snapshots) depends on this holding. */
function offsetOf(session: ClaudeSessionState): number {
  return (session as unknown as { conversationOffset?: number }).conversationOffset ?? 0;
}

describe('capInPlace', () => {
  it('keeps the most recent entries and mutates in place', () => {
    const arr = [1, 2, 3, 4, 5];
    capInPlace(arr, 3);
    expect(arr).toEqual([3, 4, 5]);
  });

  it('leaves a short array alone', () => {
    const arr = [1, 2];
    capInPlace(arr, 5);
    expect(arr).toEqual([1, 2]);
  });
});

describe('truncateToolResponse', () => {
  it('passes a response through untouched when it fits', () => {
    const small = 'x'.repeat(100);
    expect(truncateToolResponse(small)).toBe(small);
  });

  it('truncates an oversized response and says how much it dropped', () => {
    const huge = 'x'.repeat(MAX_TOOL_RESPONSE_CHARS + 5_000);
    const out = truncateToolResponse(huge) as string;
    expect(out.length).toBeLessThan(huge.length);
    expect(out).toContain('[truncated 5000 chars]');
    expect(out.startsWith('x'.repeat(1_000))).toBe(true);
  });

  it('keeps far more than anything the UI renders', () => {
    // ToolTraceCard's excerptJson shows at most 4000 chars.
    expect(MAX_TOOL_RESPONSE_CHARS).toBeGreaterThan(4_000);
  });

  it('leaves non-string payloads alone', () => {
    // Structured results are small, and rewriting them would break the shape
    // consumers switch on.
    const obj = { is_error: true, detail: 'nope' };
    expect(truncateToolResponse(obj)).toBe(obj);
    expect(truncateToolResponse(undefined)).toBeUndefined();
    expect(truncateToolResponse(null)).toBeNull();
  });

  it('bounds a pathological Read result to a fixed size', () => {
    // The measured case: a session that Read a few large files reached 6.3MB
    // and 3.1ms per clone, paid ~60 times a second for the rest of its life.
    const fiveMb = 'y'.repeat(5 * 1024 * 1024);
    const out = truncateToolResponse(fiveMb) as string;
    expect(out.length).toBeLessThan(MAX_TOOL_RESPONSE_CHARS + 100);
  });
});

describe('MAX_FILE_CHANGES', () => {
  it('is above what the renderer keeps for background snapshots', () => {
    // compactClaudeSnapshotForBackground tails fileChanges to 80; the store
    // must not be the thing that truncates first for ordinary sessions.
    expect(MAX_FILE_CHANGES).toBeGreaterThan(80);
  });
});

describe('truncateToolInput', () => {
  it('returns a small input by reference (the common case must not be rebuilt)', () => {
    const input = { file_path: '/tmp/a.ts', old_string: 'a', new_string: 'b' };
    expect(truncateToolInput(input)).toBe(input);
  });

  it("bounds a Write's whole-file content", () => {
    const input = { file_path: '/tmp/big.ts', content: 'x'.repeat(2 * 1024 * 1024) };
    const out = truncateToolInput(input) as { file_path: string; content: string };
    expect(JSON.stringify(out).length).toBeLessThan(MAX_TOOL_INPUT_CHARS + 500);
    expect(out.content).toContain('truncated');
    // The identifying field is what consumers read — it survives verbatim.
    expect(out.file_path).toBe('/tmp/big.ts');
  });

  it('keeps every path of a multi-file apply_patch while bounding the diffs', () => {
    // recordManagedFileChange walks changes[].path off the stored input, so a
    // budget spent on the first diff must not eat the later paths.
    const input = {
      changes: Array.from({ length: 5 }, (_, i) => ({
        path: `/repo/file${i}.ts`,
        kind: 'update',
        diff: 'd'.repeat(500_000),
      })),
    };
    const out = truncateToolInput(input) as { changes: { path: string; diff: string }[] };
    expect(out.changes.map((c) => c.path)).toEqual([
      '/repo/file0.ts',
      '/repo/file1.ts',
      '/repo/file2.ts',
      '/repo/file3.ts',
      '/repo/file4.ts',
    ]);
    expect(JSON.stringify(out).length).toBeLessThan(MAX_TOOL_INPUT_CHARS + 2_000);
  });

  it('leaves a structured input whose strings are all short completely alone', () => {
    // A TodoWrite/AskUserQuestion payload is many short strings; trimming those
    // would corrupt the plan for no memory gain.
    const input = {
      todos: Array.from({ length: 40 }, (_, i) => ({
        content: `step ${i}`,
        status: 'pending',
        activeForm: `doing step ${i}`,
      })),
    };
    expect(truncateToolInput(input)).toEqual(input);
  });

  it('bounds a bare string input and passes scalars through', () => {
    const huge = 'z'.repeat(MAX_TOOL_INPUT_CHARS + 1_000);
    expect((truncateToolInput(huge) as string).length).toBeLessThan(MAX_TOOL_INPUT_CHARS + 100);
    expect(truncateToolInput(null)).toBeNull();
    expect(truncateToolInput(42)).toBe(42);
  });
});

describe('capConversationInPlace', () => {
  it('banks every dropped turn into conversationOffset', () => {
    const session = {
      conversation: Array.from({ length: MAX_CONVERSATION_TURNS + 37 }, (_, i) => i),
    } as { conversation: number[]; conversationOffset?: number };
    capConversationInPlace(session);
    expect(session.conversation.length).toBe(MAX_CONVERSATION_TURNS);
    expect(session.conversationOffset).toBe(37);
    // Absolute index of the first surviving turn == what it was before the trim.
    expect(session.conversation[0]).toBe(session.conversationOffset);
  });

  it('accumulates across repeated trims', () => {
    const session = {
      conversation: Array.from({ length: MAX_CONVERSATION_TURNS + 10 }, (_, i) => i),
      conversationOffset: 100,
    } as { conversation: number[]; conversationOffset?: number };
    capConversationInPlace(session);
    expect(session.conversationOffset).toBe(110);
  });

  it('leaves a short conversation and its offset alone', () => {
    const session = { conversation: [1, 2, 3] } as {
      conversation: number[];
      conversationOffset?: number;
    };
    capConversationInPlace(session);
    expect(session.conversation).toEqual([1, 2, 3]);
    expect(session.conversationOffset).toBeUndefined();
  });

  it('resets the offset when the conversation was cleared for a rebuild', () => {
    // delta.reset / resyncConversation empty the array and replay the whole
    // transcript — the replay re-trims, so a carried offset would double-count.
    const session = { conversation: [] as number[], conversationOffset: 900 };
    resetConversationOffsetIfRebuilt(session);
    expect(session.conversationOffset).toBe(0);
  });

  it('leaves the offset alone while turns are still present', () => {
    const session = { conversation: [1], conversationOffset: 900 };
    resetConversationOffsetIfRebuilt(session);
    expect(session.conversationOffset).toBe(900);
  });

  // ClaudePane retires an optimistic bubble per user send that arrives, so it
  // needs a tally that survives a trim. The turn offset can't supply it — these
  // turns are mostly tool calls — so the user sends among the dropped turns are
  // banked separately.
  it('banks the user sends among the dropped turns, not just the turn count', () => {
    // One genuine user send every 10th turn, plus a synthetic nameless command
    // card that must NOT be counted as one.
    const conversation = Array.from({ length: MAX_CONVERSATION_TURNS + 100 }, (_, i) =>
      i % 10 === 0 ? { role: 'user' } : { role: 'assistant' },
    );
    conversation[5] = { role: 'user', command: { name: '' } } as (typeof conversation)[number];
    const session = { conversation };
    capConversationInPlace(session);

    expect(session.conversation.length).toBe(MAX_CONVERSATION_TURNS);
    expect(session.conversationOffset).toBe(100);
    // Dropped turns 0..99: indices 0,10,…,90 are user sends. Index 5 is the
    // nameless command card and is excluded.
    expect(session.conversationUserOffset).toBe(10);
  });

  it('keeps the absolute user tally moving forward across a trim', () => {
    // The property ClaudePane depends on: offset + in-window count never goes
    // backwards when the head is trimmed. A window-relative count would drop by
    // 10 here, which reads as "the thread reset".
    const conversation = Array.from({ length: MAX_CONVERSATION_TURNS + 100 }, (_, i) =>
      i % 10 === 0 ? { role: 'user' } : { role: 'assistant' },
    );
    const session: {
      conversation: typeof conversation;
      conversationOffset?: number;
      conversationUserOffset?: number;
    } = { conversation };
    const before = session.conversation.filter((t) => t.role === 'user').length;

    capConversationInPlace(session);
    const after =
      (session.conversationUserOffset ?? 0) +
      session.conversation.filter((t) => t.role === 'user').length;

    expect(after).toBe(before);
  });

  it('resets the user offset with the turn offset on a rebuild', () => {
    const session = { conversation: [], conversationOffset: 900, conversationUserOffset: 40 };
    resetConversationOffsetIfRebuilt(session);
    expect(session.conversationOffset).toBe(0);
    expect(session.conversationUserOffset).toBe(0);
  });
});

describe('conversation growth through the real ingest paths', () => {
  it('10k applied turns leave at most the cap, with the offset accounting for every drop', () => {
    const session = mkSession();
    const items: ConversationItemWire[] = Array.from({ length: 10_000 }, (_, i) => ({
      kind: 'tool_use',
      id: `toolu_${i}`,
      name: 'Bash',
      input: { command: `echo ${i}` },
      timestamp: new Date(1_700_000_000_000 + i).toISOString(),
    }));
    applyConversationItems(session, items, () => {});

    expect(session.conversation.length).toBe(MAX_CONVERSATION_TURNS);
    expect(offsetOf(session)).toBe(10_000 - MAX_CONVERSATION_TURNS);
    // Nothing was lost from the middle: absolute index still identifies the turn.
    const first = session.conversation[0].toolCalls?.[0];
    expect(first?.id).toBe(`toolu_${offsetOf(session)}`);
    const last = session.conversation[session.conversation.length - 1].toolCalls?.[0];
    expect(last?.id).toBe('toolu_9999');
  });

  it('keeps the offset consistent when the turns arrive across many batches', () => {
    const session = mkSession();
    let n = 0;
    for (let batch = 0; batch < 100; batch++) {
      applyConversationItems(
        session,
        Array.from({ length: 40 }, () => ({
          kind: 'tool_use' as const,
          id: `toolu_${n++}`,
          name: 'Bash',
          input: {},
        })),
        () => {},
      );
      // The invariant every index-anchoring consumer relies on.
      expect(offsetOf(session) + session.conversation.length).toBe(n);
      expect(session.conversation.length).toBeLessThanOrEqual(MAX_CONVERSATION_TURNS);
    }
    expect(offsetOf(session)).toBe(4_000 - MAX_CONVERSATION_TURNS);
  });

  it('a resync replaying the transcript restarts the offset instead of double-counting', () => {
    const session = mkSession();
    const batch = (from: number, count: number): ConversationItemWire[] =>
      Array.from({ length: count }, (_, i) => ({
        kind: 'tool_use',
        id: `toolu_${from + i}`,
        name: 'Bash',
        input: {},
      }));
    applyConversationItems(session, batch(0, 5_000), () => {});
    expect(offsetOf(session)).toBe(5_000 - MAX_CONVERSATION_TURNS);

    // What applyConversationDelta({reset}) / resyncConversation do before
    // replaying the daemon's full history.
    session.conversation = [];
    session.totalToolCalls = 0;
    applyConversationItems(session, batch(0, 5_000), () => {});
    expect(offsetOf(session)).toBe(5_000 - MAX_CONVERSATION_TURNS);
    expect(session.conversation[0].toolCalls?.[0].id).toBe(`toolu_${offsetOf(session)}`);
    // The replay re-trims, so the cap is a hard window rather than a cache:
    // turns older than it can never be brought back into the UI.
    expect(session.conversation.some((t) => t.toolCalls?.[0]?.id === 'toolu_0')).toBe(false);
  });

  it('Notification hooks cannot grow the conversation past the cap either', () => {
    // The hook path pushes one turn at a time — the other ingest seam into the
    // same array, and the one that historically had no bound at all.
    const session = mkSession();
    for (let i = 0; i < 3_000; i++) {
      applyHookEvent(session, { hook_event_name: 'Notification', message: `n${i}` });
    }
    expect(session.conversation.length).toBe(MAX_CONVERSATION_TURNS);
    expect(offsetOf(session)).toBe(3_000 - MAX_CONVERSATION_TURNS);
    expect(session.conversation[0].content).toBe(`n${offsetOf(session)}`);
  });

  it('bounds a session that Writes large files repeatedly', () => {
    // Isolates the other half of the finding: this stays under the turn cap on
    // purpose, so what keeps the per-flush structured clone small here is the
    // per-turn input budget alone. The turn cap is what stops the count.
    const session = mkSession();
    const body = 'x'.repeat(512 * 1024);
    for (let i = 0; i < 600; i++) {
      applyHookEvent(session, {
        hook_event_name: 'PreToolUse',
        tool_use_id: `toolu_${i}`,
        tool_name: 'Write',
        tool_input: { file_path: `/repo/f${i}.ts`, content: body },
      });
      applyConversationItems(
        session,
        [
          {
            kind: 'tool_use',
            id: `toolu_${i}`,
            name: 'Write',
            input: { file_path: `/repo/f${i}.ts`, content: body },
          },
        ],
        () => {},
      );
    }
    const cloneSize = JSON.stringify({
      conversation: session.conversation,
      fileChanges: session.fileChanges,
      activeToolCalls: session.activeToolCalls,
    }).length;
    // Without the input budget this is ~600 MB — the per-flush clone cost that
    // killed long-running sessions.
    expect(cloneSize).toBeLessThan(32 * 1024 * 1024);
    expect(session.conversation.length).toBeLessThan(MAX_CONVERSATION_TURNS);
    expect(session.fileChanges[0].input.file_path).toMatch(/^\/repo\/f\d+\.ts$/);
  });
});
