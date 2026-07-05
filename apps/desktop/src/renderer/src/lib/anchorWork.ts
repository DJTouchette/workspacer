import type {
  ConversationTurn,
  ToolCall,
  SubagentInfo,
  WorkflowRunInfo,
} from '../types/claudeSession';

/**
 * Anchor subagents/workflow runs to the Agent/Workflow tool calls that spawned
 * them so they render inline in the timeline.
 *
 * Exact joins first:
 *  - a subagent's `toolUseId` (from its meta.json, via the workflowWatcher) is
 *    the id of the Agent tool_use that spawned it;
 *  - a workflow run's `runId` appears verbatim in its Workflow call's tool
 *    result (transcript dir / script path), so a response containing the runId
 *    identifies the spawning call.
 *
 * Anything without an exact key falls back to the old order-match — the nth
 * unmatched subagent pairs with the nth unclaimed Agent call (tool_use blocks
 * land in the JSONL before execution starts, so ordering holds). Leftovers
 * (hook arrived before the transcript caught up) are returned as unanchored
 * and render in the live section at the bottom.
 */
export interface AnchoredWork {
  toolIdToSubagent: Map<string, SubagentInfo>;
  toolIdToWorkflow: Map<string, WorkflowRunInfo>;
  unanchoredSubagents: SubagentInfo[];
  unanchoredWorkflows: WorkflowRunInfo[];
}

export function anchorWork(
  conversation: ConversationTurn[],
  subagents: SubagentInfo[],
  workflows: WorkflowRunInfo[],
): AnchoredWork {
  const agentCalls: ToolCall[] = [];
  const workflowCalls: ToolCall[] = [];
  for (const turn of conversation) {
    for (const tc of turn.toolCalls ?? []) {
      if (tc.name === 'Agent') agentCalls.push(tc);
      else if (tc.name === 'Workflow') workflowCalls.push(tc);
    }
  }

  // ── Subagents: exact join on toolUseId, order-match the rest ──
  const toolIdToSubagent = new Map<string, SubagentInfo>();
  const agentCallIds = new Set(agentCalls.map((c) => c.id));
  const orderMatchedSubs: SubagentInfo[] = [];
  for (const sub of subagents) {
    if (sub.toolUseId && agentCallIds.has(sub.toolUseId) && !toolIdToSubagent.has(sub.toolUseId)) {
      toolIdToSubagent.set(sub.toolUseId, sub);
    } else {
      orderMatchedSubs.push(sub);
    }
  }
  const freeAgentCalls = agentCalls.filter((c) => !toolIdToSubagent.has(c.id));
  orderMatchedSubs.forEach((sub, i) => {
    if (i < freeAgentCalls.length) toolIdToSubagent.set(freeAgentCalls[i].id, sub);
  });
  const unanchoredSubagents = orderMatchedSubs.slice(freeAgentCalls.length);

  // ── Workflow runs: exact join on runId-in-response, order-match the rest ──
  const toolIdToWorkflow = new Map<string, WorkflowRunInfo>();
  const orderMatchedRuns: WorkflowRunInfo[] = [];
  for (const run of workflows) {
    const call = workflowCalls.find(
      (c) =>
        !toolIdToWorkflow.has(c.id) &&
        typeof c.response === 'string' &&
        c.response.includes(run.runId),
    );
    if (call) toolIdToWorkflow.set(call.id, run);
    else orderMatchedRuns.push(run);
  }
  const freeWorkflowCalls = workflowCalls.filter((c) => !toolIdToWorkflow.has(c.id));
  orderMatchedRuns.forEach((run, i) => {
    if (i < freeWorkflowCalls.length) toolIdToWorkflow.set(freeWorkflowCalls[i].id, run);
  });
  const unanchoredWorkflows = orderMatchedRuns.slice(freeWorkflowCalls.length);

  return { toolIdToSubagent, toolIdToWorkflow, unanchoredSubagents, unanchoredWorkflows };
}
