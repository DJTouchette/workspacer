import { expect, it } from 'vitest';
import {
  createConversationIndexer,
  reconcileConversationTurns,
} from '../src/lib/conversationIndex';
import { countUserSends } from '../../main/shared/conversationCount';
import { anchorWork } from '../src/lib/anchorWork';
import type { ConversationTurn, ToolCall } from '../src/types/claudeSession';
const tool = (id: string, name = 'Read'): ToolCall => ({
  id,
  name,
  input: { path: '/repo/file' },
  startedAt: 1,
  status: 'running',
});
const turn = (content: string, calls?: ToolCall[]): ConversationTurn => ({
  role: 'assistant',
  content,
  timestamp: 1,
  toolCalls: calls,
});
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));

it('reuses transport clones but preserves earlier corrections and tool completions', () => {
  const previous = [turn('first', [tool('a')]), turn('tail')];
  expect(reconcileConversationTurns(previous, clone(previous))).toBe(previous);
  const incoming = clone(previous);
  incoming[1].content += ' streamed';
  const streamed = reconcileConversationTurns(previous, incoming);
  expect(streamed[0]).toBe(previous[0]);
  expect(streamed[1].toolCalls).toBe(previous[1].toolCalls);
  incoming[0].toolCalls![0].status = 'complete';
  incoming[0].toolCalls![0].response = { nested: ['result'] };
  const corrected = reconcileConversationTurns(previous, incoming);
  expect(corrected[0]).not.toBe(previous[0]);
  expect(corrected[0].toolCalls![0].response).toEqual({ nested: ['result'] });
});

it('retains tool and orchestration indexes across text updates', () => {
  const index = createConversationIndexer();
  const previous = [turn('first', [tool('a', 'Agent')]), turn('tail', [tool('r')])];
  const before = index(previous);
  const incoming = clone(previous);
  incoming[1].content += ' streamed';
  expect(index(reconcileConversationTurns(previous, incoming))).toBe(before);
});

it('matches fresh derivation through append, earlier mutation, duplicate tools, trim and restart', () => {
  const index = createConversationIndexer();
  const user: ConversationTurn = { role: 'user', content: 'hello', timestamp: 1 };
  const orphan: ConversationTurn = { ...user, command: { name: '' } };
  const a = tool('a', 'Agent');
  const wf = { ...tool('wf', 'Workflow'), response: 'run-1' };
  const variants = [
    [user, turn('first', [a]), orphan],
    [user, turn('first', [a]), orphan, turn('tail', [a, wf])],
    [user, turn('corrected', [{ ...a, status: 'complete' as const }]), orphan, turn('tail', [wf])],
    [orphan, turn('tail', [wf])],
    [user],
    [],
  ];
  for (const turns of variants) {
    const actual = index(turns);
    expect(actual.userCount).toBe(countUserSends(turns));
    expect([...actual.toolIds].sort()).toEqual(
      [...new Set(turns.flatMap((t) => t.toolCalls?.map((c) => c.id) ?? []))].sort(),
    );
    expect(anchorWork([], [], [], actual)).toEqual(anchorWork(turns, [], []));
    expect(actual.agentCalls).toEqual(
      turns.flatMap((t) => t.toolCalls?.filter((c) => c.name === 'Agent') ?? []),
    );
    expect(actual.workflowCalls).toEqual(
      turns.flatMap((t) => t.toolCalls?.filter((c) => c.name === 'Workflow') ?? []),
    );
  }
});
