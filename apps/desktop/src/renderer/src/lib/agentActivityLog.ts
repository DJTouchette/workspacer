/**
 * Recent-activity feed for the sidebar agent cards: the last few things the
 * agent actually did — tool calls (running or finished) and assistant
 * messages — merged into one time-ordered list.
 *
 * Tool calls are unioned from three sources because none alone survives:
 * the hook-driven activeToolCalls/completedToolCalls lists are cleared at
 * every turn end (applyStopEvent), while the conversation turns keep their
 * transcript-derived toolCalls forever but lag the hooks by a beat. Dedup
 * is by tool_use id.
 */
import type { ClaudeSessionSnapshot, ToolCall } from '../types/claudeSession';
import { formatToolSummary } from '../components/claude-shared';

export interface ActivityLine {
  text: string;
  /** Epoch ms used for ordering (0 when the source carried no timestamp). */
  at: number;
  kind: 'tool' | 'tool-running' | 'message';
}

/** First line of a message, stripped of leading markdown furniture. */
function firstLine(content: string): string {
  return content
    .trim()
    .split('\n')[0]
    .replace(/^[#>*\-\s`]+/, '');
}

export function collectRecentActivity(
  snap: ClaudeSessionSnapshot | undefined,
  max = 3,
): ActivityLine[] {
  if (!snap) return [];

  const toolsById = new Map<string, ToolCall>();
  for (const turn of snap.conversation ?? []) {
    for (const tc of turn.toolCalls ?? []) {
      if (tc.id) toolsById.set(tc.id, tc);
    }
  }
  // Hook-driven entries win the dedup — they're fresher (a tool appears here
  // the moment PreToolUse fires, before the transcript tailer catches up).
  for (const tc of snap.completedToolCalls ?? []) {
    if (tc.id) toolsById.set(tc.id, tc);
  }
  for (const tc of snap.activeToolCalls ?? []) {
    if (tc.id) toolsById.set(tc.id, tc);
  }

  const lines: ActivityLine[] = [];
  for (const tc of toolsById.values()) {
    lines.push({
      text: formatToolSummary(tc).call,
      at: tc.completedAt ?? tc.startedAt ?? 0,
      kind: tc.status === 'running' ? 'tool-running' : 'tool',
    });
  }
  for (const turn of snap.conversation ?? []) {
    if (turn.role !== 'assistant') continue;
    if (turn.command) continue; // slash-command runs aren't the agent talking
    const text = firstLine(turn.content ?? '');
    if (text) lines.push({ text, at: turn.timestamp ?? 0, kind: 'message' });
  }

  lines.sort((a, b) => a.at - b.at);
  return lines.slice(-max);
}
