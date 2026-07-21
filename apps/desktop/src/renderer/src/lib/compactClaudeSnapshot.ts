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

function compactToolCall(tool: ToolCall): ToolCall {
  return {
    ...tool,
    input: compactPayload(tool.input),
    response: compactPayload(tool.response),
  };
}

function compactConversationTurn(turn: ConversationTurn): ConversationTurn {
  return {
    ...turn,
    content: truncateString(turn.content ?? ''),
    toolCalls: turn.toolCalls?.map(compactToolCall),
  };
}

function compactFileChange(change: FileChange): FileChange {
  return {
    ...change,
    input: compactPayload(change.input, 1000),
  };
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
