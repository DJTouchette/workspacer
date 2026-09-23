import type { ConversationTurn, ToolCall } from '../types/claudeSession';
import { isUserSend } from '../../../main/shared/conversationCount';

/** IPC payloads are JSON-shaped. Compare every field, including earlier tool
 * completions/replay corrections, rather than assuming only the tail changes. */
function sameValue(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a) && Array.isArray(b) && a.length !== b.length) return false;
  const left = a as Record<string, unknown>;
  const right = b as Record<string, unknown>;
  for (const key of Object.keys(left)) if (!sameValue(left[key], right[key])) return false;
  for (const key of Object.keys(right)) {
    if (!Object.prototype.hasOwnProperty.call(left, key) && right[key] !== undefined) return false;
  }
  return true;
}

/** Invoke only within one session/offset. Reuses settled turns and tool arrays
 * so React and derived indexes can distinguish new work from transport clones. */
export function reconcileConversationTurns(
  previous: ConversationTurn[] | undefined,
  next: ConversationTurn[],
): ConversationTurn[] {
  if (!previous || previous === next) return next;
  let unchanged = previous.length === next.length;
  const result = next.map((turn, i) => {
    const old = previous[i];
    if (old && sameValue(old, turn)) return old;
    unchanged = false;
    if (old && sameValue(old.toolCalls, turn.toolCalls))
      return { ...turn, toolCalls: old.toolCalls };
    return turn;
  });
  return unchanged ? previous : result;
}

export interface ConversationIndex {
  userCount: number;
  toolIds: ReadonlySet<string>;
  agentCalls: ToolCall[];
  workflowCalls: ToolCall[];
}

/** Keeps counts and orchestration inventories stable during text streaming.
 * One identity walk replaces multiple full tool/history walks. Any changed
 * earlier turn, reset, prepend or trim is handled just like a changed tail. */
export function createConversationIndexer() {
  let previous: ConversationTurn[] = [];
  let userCount = 0;
  const counts = new Map<string, number>();
  let result: ConversationIndex = {
    userCount: 0,
    toolIds: new Set(),
    agentCalls: [],
    workflowCalls: [],
  };
  return (turns: ConversationTurn[]): ConversationIndex => {
    if (turns === previous) return result;
    let toolsChanged = false;
    let anchorsChanged = false;
    const adjust = (tools: ToolCall[] | undefined, amount: number) => {
      for (const tool of tools ?? []) {
        const count = (counts.get(tool.id) ?? 0) + amount;
        if (count) counts.set(tool.id, count);
        else counts.delete(tool.id);
        if (tool.name === 'Agent' || tool.name === 'Workflow') anchorsChanged = true;
      }
    };
    for (let i = 0; i < Math.max(previous.length, turns.length); i++) {
      const old = previous[i];
      const next = turns[i];
      if (old === next) continue;
      userCount += Number(!!next && isUserSend(next)) - Number(!!old && isUserSend(old));
      if (old?.toolCalls !== next?.toolCalls) {
        toolsChanged = true;
        adjust(old?.toolCalls, -1);
        adjust(next?.toolCalls, 1);
      }
    }
    let { agentCalls, workflowCalls } = result;
    if (anchorsChanged) {
      agentCalls = [];
      workflowCalls = [];
      for (const turn of turns)
        for (const call of turn.toolCalls ?? []) {
          if (call.name === 'Agent') agentCalls.push(call);
          else if (call.name === 'Workflow') workflowCalls.push(call);
        }
    }
    if (userCount !== result.userCount || toolsChanged || anchorsChanged) {
      result = {
        userCount,
        toolIds: toolsChanged ? new Set(counts.keys()) : result.toolIds,
        agentCalls,
        workflowCalls,
      };
    }
    previous = turns;
    return result;
  };
}
