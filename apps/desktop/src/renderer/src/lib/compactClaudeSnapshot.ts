import type {
  ClaudeSessionSnapshot,
  ConversationTurn,
  FileChange,
  PendingApproval,
  ToolCall,
} from '../types/claudeSession';

const MAX_BACKGROUND_CONVERSATION_TURNS = 12;
const MAX_BACKGROUND_COMPLETED_TOOLS = 20;
const MAX_BACKGROUND_ACTIVE_TOOLS = 20;
const MAX_BACKGROUND_FILE_CHANGES = 80;
const MAX_TEXT_CHARS = 4000;
const MAX_PAYLOAD_CHARS = 2000;

function tail<T>(items: T[], max: number): T[] {
  return items.length > max ? items.slice(items.length - max) : items.slice();
}

function truncateString(value: string, maxChars = MAX_TEXT_CHARS): string {
  if (value.length <= maxChars) return value;
  return `${value.slice(0, maxChars)}\n[truncated ${value.length - maxChars} chars]`;
}

function compactPayload(value: unknown, maxChars = MAX_PAYLOAD_CHARS): unknown {
  if (typeof value === 'string') return truncateString(value, maxChars);
  if (value === null || value === undefined) return value;
  try {
    const json = JSON.stringify(value);
    if (!json || json.length <= maxChars) return value;
    return {
      __workspacerTruncated: true,
      originalChars: json.length,
      preview: truncateString(json, maxChars),
    };
  } catch {
    return {
      __workspacerTruncated: true,
      preview: String(value).slice(0, maxChars),
    };
  }
}

/**
 * Memo for entries whose compacted form can never change again.
 *
 * This runs on every `claude-session:update`, which is up to ~60/s per
 * streaming session, over a tail of up to 80 file changes and 20 completed
 * tool calls — and `compactPayload` JSON.stringifies each one just to measure
 * it. The overwhelming majority of that work re-derives an identical result for
 * an entry that was already final several hundred ticks ago.
 *
 * Object identity can't be the key: snapshots arrive over IPC, so every tick
 * delivers structurally-cloned objects with fresh identities and a WeakMap
 * would never hit. These entries carry their own immutable identity instead.
 */
const MAX_MEMO_ENTRIES = 1024;
const compactMemo = new Map<string, unknown>();

function memoized<T>(key: string, compute: () => T): T {
  const hit = compactMemo.get(key);
  if (hit !== undefined) return hit as T;
  const value = compute();
  compactMemo.set(key, value);
  if (compactMemo.size > MAX_MEMO_ENTRIES) {
    // Evict a chunk at a time; Map iterates in insertion order, so this drops
    // the oldest quarter rather than paying an eviction on every insert.
    let toDrop = MAX_MEMO_ENTRIES / 4;
    for (const k of compactMemo.keys()) {
      compactMemo.delete(k);
      if (--toDrop <= 0) break;
    }
  }
  return value;
}

function compactToolCall(tool: ToolCall): ToolCall {
  const compute = (): ToolCall => ({
    ...tool,
    input: compactPayload(tool.input),
    response: compactPayload(tool.response),
  });
  // A running tool's response is still filling in, so only settled calls are
  // safe to memo — and those are the ones that pile up.
  if (tool.status === 'running') return compute();
  return memoized(`t|${tool.id}|${tool.status}|${tool.completedAt ?? 0}`, compute);
}

function compactConversationTurn(turn: ConversationTurn): ConversationTurn {
  return {
    ...turn,
    content: truncateString(turn.content ?? ''),
    toolCalls: turn.toolCalls?.map(compactToolCall),
  };
}

function compactFileChange(change: FileChange): FileChange {
  // A recorded file change is immutable — it's a hook event that already fired.
  return memoized(`f|${change.timestamp}|${change.toolName}|${change.path}`, () => ({
    ...change,
    input: compactPayload(change.input, 1000),
  }));
}

function compactPendingApproval(approval: PendingApproval | null): PendingApproval | null {
  if (!approval) return null;
  return {
    ...approval,
    toolInput: compactPayload(approval.toolInput),
  };
}

/**
 * Keep background/global session snapshots bounded.
 *
 * Active Claude panes still request and retain the full snapshot. Sidebar,
 * Fleet Deck, Triage Inbox, and hidden panes only need recent context and
 * attention metadata, so holding the whole transcript/tool payload there
 * multiplies memory use across long-running sessions.
 */
export function compactClaudeSnapshotForBackground(
  snapshot: ClaudeSessionSnapshot,
): ClaudeSessionSnapshot {
  const fullConversation = snapshot.conversation ?? [];
  const keptConversation = tail(fullConversation, MAX_BACKGROUND_CONVERSATION_TURNS);
  return {
    ...snapshot,
    conversation: keptConversation.map(compactConversationTurn),
    // Accumulates across repeated compaction so global turn indices
    // (conversationOffset + array index) stay stable for consumers that key
    // or anchor by index (ClaudePane's conversation keys, turn snapshots).
    conversationOffset:
      (snapshot.conversationOffset ?? 0) + (fullConversation.length - keptConversation.length),
    activeToolCalls: tail(snapshot.activeToolCalls ?? [], MAX_BACKGROUND_ACTIVE_TOOLS).map(
      compactToolCall,
    ),
    completedToolCalls: tail(snapshot.completedToolCalls ?? [], MAX_BACKGROUND_COMPLETED_TOOLS).map(
      compactToolCall,
    ),
    fileChanges: tail(snapshot.fileChanges ?? [], MAX_BACKGROUND_FILE_CHANGES).map(
      compactFileChange,
    ),
    pendingApproval: compactPendingApproval(snapshot.pendingApproval),
  };
}
