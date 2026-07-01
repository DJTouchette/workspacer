import { describe, it, expect } from 'vitest';
import { anchorWork } from '../src/lib/anchorWork';
import type { ConversationTurn, ToolCall, SubagentInfo, WorkflowRunInfo } from '../src/types/claudeSession';

function call(id: string, name: string, response?: string): ToolCall {
  return { id, name, input: {}, response, status: 'complete', startedAt: 0 };
}

function turnWith(...calls: ToolCall[]): ConversationTurn {
  return { role: 'assistant', content: '', timestamp: 0, toolCalls: calls };
}

function sub(id: string, toolUseId?: string): SubagentInfo {
  return { id, type: 'general-purpose', status: 'running', startedAt: 0, toolUseId };
}

function run(runId: string): WorkflowRunInfo {
  return { runId, status: 'running', startedAt: 0, phases: [], agents: [] };
}

describe('anchorWork', () => {
  it('anchors subagents exactly by toolUseId, regardless of order', () => {
    const conversation = [turnWith(call('toolu_1', 'Agent'), call('toolu_2', 'Agent'))];
    // Reported in reverse order — order-matching alone would swap them.
    const subagents = [sub('b', 'toolu_2'), sub('a', 'toolu_1')];
    const { toolIdToSubagent, unanchoredSubagents } = anchorWork(conversation, subagents, []);
    expect(toolIdToSubagent.get('toolu_1')?.id).toBe('a');
    expect(toolIdToSubagent.get('toolu_2')?.id).toBe('b');
    expect(unanchoredSubagents).toEqual([]);
  });

  it('falls back to order-matching for subagents without a toolUseId', () => {
    const conversation = [turnWith(call('toolu_1', 'Agent'), call('toolu_2', 'Agent'))];
    const subagents = [sub('a'), sub('b')];
    const { toolIdToSubagent, unanchoredSubagents } = anchorWork(conversation, subagents, []);
    expect(toolIdToSubagent.get('toolu_1')?.id).toBe('a');
    expect(toolIdToSubagent.get('toolu_2')?.id).toBe('b');
    expect(unanchoredSubagents).toEqual([]);
  });

  it('order-matches unkeyed subagents only against unclaimed calls', () => {
    const conversation = [turnWith(call('toolu_1', 'Agent'), call('toolu_2', 'Agent'))];
    // 'b' exactly claims the FIRST call; unkeyed 'a' must take the second,
    // not double-book the first.
    const subagents = [sub('a'), sub('b', 'toolu_1')];
    const { toolIdToSubagent } = anchorWork(conversation, subagents, []);
    expect(toolIdToSubagent.get('toolu_1')?.id).toBe('b');
    expect(toolIdToSubagent.get('toolu_2')?.id).toBe('a');
  });

  it('returns leftover subagents as unanchored when calls run out', () => {
    const conversation = [turnWith(call('toolu_1', 'Agent'))];
    const subagents = [sub('a'), sub('b')];
    const { unanchoredSubagents } = anchorWork(conversation, subagents, []);
    expect(unanchoredSubagents.map(s => s.id)).toEqual(['b']);
  });

  it('anchors workflow runs by runId found in the call response', () => {
    const conversation = [
      turnWith(call('toolu_1', 'Workflow', 'Task ID: x\nTranscript dir: /s/subagents/workflows/wf_aaa-111')),
      turnWith(call('toolu_2', 'Workflow', 'Task ID: y\nTranscript dir: /s/subagents/workflows/wf_bbb-222')),
    ];
    // Watcher order (startedAt) reversed vs call order.
    const workflows = [run('wf_bbb-222'), run('wf_aaa-111')];
    const { toolIdToWorkflow, unanchoredWorkflows } = anchorWork(conversation, [], workflows);
    expect(toolIdToWorkflow.get('toolu_1')?.runId).toBe('wf_aaa-111');
    expect(toolIdToWorkflow.get('toolu_2')?.runId).toBe('wf_bbb-222');
    expect(unanchoredWorkflows).toEqual([]);
  });

  it('order-matches runs whose call has no response yet', () => {
    // Response hasn't landed (tool_result not yet streamed) — fall back to order.
    const conversation = [turnWith(call('toolu_1', 'Workflow'))];
    const workflows = [run('wf_aaa-111')];
    const { toolIdToWorkflow, unanchoredWorkflows } = anchorWork(conversation, [], workflows);
    expect(toolIdToWorkflow.get('toolu_1')?.runId).toBe('wf_aaa-111');
    expect(unanchoredWorkflows).toEqual([]);
  });

  it('leaves a run unanchored when there are no Workflow calls', () => {
    const workflows = [run('wf_aaa-111')];
    const { unanchoredWorkflows } = anchorWork([], [], workflows);
    expect(unanchoredWorkflows.map(w => w.runId)).toEqual(['wf_aaa-111']);
  });

  it('ignores non-Agent/Workflow tool calls entirely', () => {
    const conversation = [turnWith(call('toolu_0', 'Bash'), call('toolu_1', 'Agent'))];
    const { toolIdToSubagent } = anchorWork(conversation, [sub('a')], []);
    expect(toolIdToSubagent.has('toolu_0')).toBe(false);
    expect(toolIdToSubagent.get('toolu_1')?.id).toBe('a');
  });
});
