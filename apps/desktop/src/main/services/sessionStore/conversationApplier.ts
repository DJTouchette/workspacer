import type { ClaudeSessionState, ToolCall } from '../claudeSessionStore';
import { applyStopEvent } from './hookEventRouter';

// ── Conversation delta application ───────────────────────────────────────────
//
// claudemon owns transcript parsing now: it tails each session's JSONL and
// streams typed items over `/conversation/stream`. This module folds those
// items into the session state — the successor to the old transcriptParser
// (which re-read the whole JSONL on every hook event in this process).

/** Wire shape of one item from claudemon's ConversationItem enum. */
export interface ConversationItemWire {
  kind: 'user_message' | 'assistant_text' | 'tool_use' | 'tool_result' | 'usage';
  // user_message / assistant_text
  text?: string;
  // tool_use
  id?: string;
  name?: string;
  input?: any;
  // tool_result
  tool_use_id?: string;
  content?: string;
  is_error?: boolean;
  // usage
  model?: string;
  usage?: any;
  message_id?: string;
  // all
  timestamp?: string;
}

/** Wire shape of one frame from `/conversation/stream`. */
export interface ConversationDeltaWire {
  session_id: string;
  seq: number;
  reset: boolean;
  items: ConversationItemWire[];
}

export type ApplyUsageFn = (
  session: ClaudeSessionState,
  model: string | null,
  usage: any,
  key: string | null,
) => void;

/** Check if a message was already added to avoid duplicates (claude's JSONL
 *  occasionally repeats a message, e.g. around compaction). */
export function isDuplicateMessage(
  session: ClaudeSessionState,
  role: string,
  content: string,
): boolean {
  if (!content) return false;
  const recent = session.conversation.slice(-5);
  return recent.some((t) => t.role === role && t.content && t.content === content);
}

/**
 * Claude Code writes these markers into the transcript when the user
 * interrupts a turn (Esc): a plain `[Request interrupted by user]` user row,
 * or `[Request interrupted by user for tool use]` as the pending tool's
 * result. Crucially, *no Stop hook fires on interrupt* — so without spotting
 * these, an interrupted session stays stuck on 'streaming' until its next
 * prompt.
 */
function isInterruptMarker(item: ConversationItemWire): boolean {
  const text =
    item.kind === 'user_message'
      ? item.text
      : item.kind === 'tool_result'
        ? item.content
        : undefined;
  return typeof text === 'string' && text.trimStart().startsWith('[Request interrupted by user');
}

function tsOf(item: ConversationItemWire): number {
  if (item.timestamp) {
    const ms = Date.parse(item.timestamp);
    if (!Number.isNaN(ms)) return ms;
  }
  return Date.now();
}

/**
 * Fold a batch of conversation items into the session, mutating it in place
 * (same contract as hookEventRouter: caller owns side-effects like pushUpdate).
 */
export function applyConversationItems(
  session: ClaudeSessionState,
  items: ConversationItemWire[],
  applyUsageFn: ApplyUsageFn,
): void {
  for (const item of items) {
    switch (item.kind) {
      case 'user_message': {
        const text = item.text ?? '';
        if (text && !isDuplicateMessage(session, 'user', text)) {
          session.conversation.push({ role: 'user', content: text, timestamp: tsOf(item) });
        }
        break;
      }

      case 'assistant_text': {
        const text = item.text ?? '';
        if (!text) break;
        // Managed adapters stream assistant text into one bubble, but in two
        // different shapes: Codex emits incremental deltas ("What", " can", …)
        // while OpenCode re-sends the full accumulated text each update ("Hello",
        // "Hello world"). Coalesce both: if the new text extends the current
        // bubble it's a growing snapshot → replace; otherwise it's a delta →
        // append. (Pushing a message per fragment renders one word per line;
        // appending snapshots would duplicate.) Claude emits whole text blocks
        // and re-emits them around compaction, so it keeps the dedup-and-push
        // path.
        const streaming = !!session.provider && session.provider !== 'claude';
        const last = session.conversation[session.conversation.length - 1];
        if (streaming && last && last.role === 'assistant' && !last.toolCalls?.length) {
          if (last.content && text.startsWith(last.content)) {
            last.content = text; // full-snapshot growth (OpenCode)
          } else {
            last.content += text; // incremental delta (Codex)
          }
        } else if (!isDuplicateMessage(session, 'assistant', text)) {
          session.conversation.push({ role: 'assistant', content: text, timestamp: tsOf(item) });
        }
        break;
      }

      case 'tool_use': {
        const ts = tsOf(item);
        const tc: ToolCall = {
          id: item.id || `tc-${ts}-${Math.random().toString(36).slice(2, 6)}`,
          name: item.name ?? 'unknown',
          input: item.input ?? {},
          status: 'complete',
          startedAt: ts,
          completedAt: ts,
        };
        session.totalToolCalls++;
        // Each tool call is its own turn — interlaced with text in timeline order
        session.conversation.push({
          role: 'assistant',
          content: '',
          timestamp: ts,
          toolCalls: [tc],
        });
        break;
      }

      case 'tool_result': {
        if (!item.tool_use_id) break;
        // Attach to the matching tool call (scan backwards — results follow
        // their calls closely)
        for (let i = session.conversation.length - 1; i >= 0; i--) {
          const tcs = session.conversation[i].toolCalls;
          if (!tcs) continue;
          const tc = tcs.find((t) => t.id === item.tool_use_id);
          if (tc) {
            tc.response = item.content ?? '';
            if (item.is_error) tc.status = 'failed';
            break;
          }
        }
        break;
      }

      case 'usage':
        applyUsageFn(session, item.model ?? null, item.usage ?? {}, item.message_id ?? null);
        break;
    }
  }

  // Housekeeping: drop hook-tracked tool calls already absorbed into
  // conversation turns, so the live work log doesn't duplicate the timeline.
  // The transcript is authoritative, so this also reaps *active* calls whose
  // PostToolUse hook was dropped (e.g. an SSE reconnect) — otherwise their
  // spinners would orphan at the bottom until the next Stop.
  if (
    items.length > 0 &&
    (session.completedToolCalls.length > 0 || session.activeToolCalls.length > 0)
  ) {
    const convToolIds = new Set<string>();
    for (const turn of session.conversation) {
      if (turn.toolCalls) for (const tc of turn.toolCalls) convToolIds.add(tc.id);
    }
    session.completedToolCalls = session.completedToolCalls.filter((tc) => !convToolIds.has(tc.id));
    session.activeToolCalls = session.activeToolCalls.filter((tc) => !convToolIds.has(tc.id));
  }

  // Interrupt detection: if the batch *ends* on an interrupt marker, the turn
  // was cancelled and no Stop hook is coming — fold in the same cleanup Stop
  // would have done (idle, clear pendings + work log). Only the trailing item
  // counts: an interrupt mid-batch is history the session already moved past
  // (e.g. a full resync replaying an old interrupt), and any follow-up prompt
  // flips the state back to 'streaming' via its UserPromptSubmit hook anyway.
  const lastMeaningful = [...items].reverse().find((i) => i.kind !== 'usage');
  if (lastMeaningful && isInterruptMarker(lastMeaningful)) {
    applyStopEvent(session);
  }
}
