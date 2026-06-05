import * as fs from 'fs';
import {
  type SessionUsage,
  contextTokensOf,
  contextLimitFor,
  turnCostUSD,
  emptyUsage,
} from '../modelUsage';
import type { ClaudeSessionState, ToolCall } from '../claudeSessionStore';

// ── Types re-exported from the root module (no duplication) ──────────────────
// (ClaudeSessionState and ToolCall are defined in claudeSessionStore.ts)

// ── Dedup helper ─────────────────────────────────────────────────────────────

/** Check if a message was already added to avoid duplicates */
export function isDuplicateMessage(session: ClaudeSessionState, role: string, content: string): boolean {
  if (!content) return false;
  const recent = session.conversation.slice(-5);
  return recent.some(
    (t) => t.role === role && t.content && t.content === content,
  );
}

// ── Entry processor ───────────────────────────────────────────────────────────

export function processTranscriptEntry(
  session: ClaudeSessionState,
  entry: any,
  applyUsageFn: (session: ClaudeSessionState, model: string | null, usage: any, key: string | null) => void,
): void {
  const type = entry.type;
  const msg = entry.message;

  if (type === 'user' && msg) {
    const contentBlocks = Array.isArray(msg.content) ? msg.content : [];
    // Skip tool_result entries — they're API plumbing, not user messages
    const hasToolResult = contentBlocks.some((b: any) => b.type === 'tool_result');
    if (hasToolResult) {
      // Extract tool results and attach to the corresponding tool calls
      for (const block of contentBlocks) {
        if (block.type === 'tool_result' && block.tool_use_id) {
          const resultText = typeof block.content === 'string'
            ? block.content
            : Array.isArray(block.content)
              ? block.content.filter((b: any) => b.type === 'text').map((b: any) => b.text).join('\n')
              : '';
          // Find the matching tool call and set its response
          for (let i = session.conversation.length - 1; i >= 0; i--) {
            const tcs = session.conversation[i].toolCalls;
            if (!tcs) continue;
            const tc = tcs.find(t => t.id === block.tool_use_id);
            if (tc) {
              tc.response = resultText;
              break;
            }
          }
        }
      }
      return;
    }

    // Real user message
    const content = typeof msg.content === 'string'
      ? msg.content
      : contentBlocks.filter((b: any) => b.type === 'text').map((b: any) => b.text).join('\n');
    if (content && !isDuplicateMessage(session, 'user', content)) {
      session.conversation.push({ role: 'user', content, timestamp: Date.now() });
    }
  } else if (type === 'assistant' && msg) {
    // Token usage rides on the assistant message. Context fullness is a
    // point-in-time snapshot (overwrite); cost/totals accumulate per message.
    if (msg.usage) {
      applyUsageFn(session, msg.model ?? null, msg.usage, entry?.message?.id ?? entry?.uuid ?? null);
    }

    // The JSONL transcript streams each content block as a separate entry.
    // Keep each block as its own conversation turn so text and tool calls
    // render interlaced in timeline order.
    const blocks = Array.isArray(msg.content) ? msg.content : [];

    for (const block of blocks) {
      if (block.type === 'thinking') continue;

      if (block.type === 'text' && block.text) {
        const text = block.text.trim();
        if (!text) continue;
        if (!isDuplicateMessage(session, 'assistant', text)) {
          session.conversation.push({
            role: 'assistant',
            content: text,
            timestamp: Date.now(),
          });
        }
      } else if (block.type === 'tool_use') {
        const tc: ToolCall = {
          id: block.id ?? `tc-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
          name: block.name ?? 'unknown',
          input: block.input ?? {},
          status: 'complete',
          startedAt: Date.now(),
          completedAt: Date.now(),
        };
        session.totalToolCalls++;

        // Each tool call is its own turn — interlaced with text
        session.conversation.push({
          role: 'assistant',
          content: '',
          timestamp: Date.now(),
          toolCalls: [tc],
        });
      }
    }
  }
}

// ── JSONL reader ─────────────────────────────────────────────────────────────

/** Read new lines from JSONL transcript and update conversation */
export function refreshFromTranscript(
  session: ClaudeSessionState,
  applyUsageFn: (session: ClaudeSessionState, model: string | null, usage: any, key: string | null) => void,
): void {
  if (!session.transcriptPath) return;
  try {
    if (!fs.existsSync(session.transcriptPath)) return;
    const content = fs.readFileSync(session.transcriptPath, 'utf-8');
    const lines = content.split('\n').filter((l: string) => l.trim());

    if (lines.length <= session.lastTranscriptLine) return;

    const newLines = lines.slice(session.lastTranscriptLine);

    let parsed = 0;
    for (const line of newLines) {
      try {
        const entry = JSON.parse(line);
        processTranscriptEntry(session, entry, applyUsageFn);
        parsed++;
      } catch {
        // Stop at first unparseable line — likely a partial write at EOF.
        // It will be re-read (now complete) on the next hook event.
        break;
      }
    }
    session.lastTranscriptLine += parsed;

    // Housekeeping: drop completedToolCalls already absorbed into conversation
    if (parsed > 0 && session.completedToolCalls.length > 0) {
      const convToolIds = new Set<string>();
      for (const turn of session.conversation) {
        if (turn.toolCalls) for (const tc of turn.toolCalls) convToolIds.add(tc.id);
      }
      session.completedToolCalls = session.completedToolCalls.filter(tc => !convToolIds.has(tc.id));
    }
  } catch (err) {
    console.error('[SessionStore] transcript read error:', err);
  }
}
