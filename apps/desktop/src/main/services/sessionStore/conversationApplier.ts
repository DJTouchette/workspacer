import type { ClaudeSessionState, PlanStep, ToolCall } from '../claudeSessionStore';
import { applyStopEvent } from './hookEventRouter';

// ── Conversation delta application ───────────────────────────────────────────
//
// claudemon owns transcript parsing now: it tails each session's JSONL and
// streams typed items over `/conversation/stream`. This module folds those
// items into the session state — the successor to the old transcriptParser
// (which re-read the whole JSONL on every hook event in this process).

/** Wire shape of one plan step. Tolerant of both activeForm / active_form. */
interface PlanStepWire {
  content?: string;
  status?: string;
  activeForm?: string;
  active_form?: string;
}

/** Wire shape of one item from claudemon's ConversationItem enum. */
export interface ConversationItemWire {
  kind:
    | 'user_message'
    | 'assistant_text'
    | 'tool_use'
    | 'tool_result'
    | 'usage'
    | 'plan'
    | 'slash_command'
    | 'command_output';
  /** Some daemons/items tag the discriminant as `type` rather than `kind`. */
  type?: string;
  // user_message / assistant_text
  text?: string;
  // tool_use / slash_command (command name, without the leading slash)
  id?: string;
  name?: string;
  input?: any;
  // slash_command / command_output
  args?: string;
  output?: string;
  // tool_result (is_error is shared by command_output)
  tool_use_id?: string;
  content?: string;
  is_error?: boolean;
  // usage
  model?: string;
  usage?: any;
  message_id?: string;
  /** True for a subagent (isSidechain) turn's usage — counts toward totals,
   *  never toward the main thread's context gauge. */
  sidechain?: boolean;
  // plan (tolerant of updatedAt / updated_at)
  steps?: PlanStepWire[];
  updatedAt?: number | string;
  updated_at?: number | string;
  // all
  timestamp?: string;
}

/** Normalize wire plan steps into the stored shape, dropping empty rows and
 *  coalescing the two casings the daemon may send for the "active form" line. */
function normalizePlanSteps(raw: PlanStepWire[] | undefined): PlanStep[] {
  if (!Array.isArray(raw)) return [];
  const steps: PlanStep[] = [];
  for (const s of raw) {
    const content = typeof s?.content === 'string' ? s.content : '';
    if (!content) continue;
    const status = s.status === 'in_progress' || s.status === 'completed' ? s.status : 'pending';
    const activeForm = s.activeForm ?? s.active_form;
    steps.push(activeForm ? { content, status, activeForm } : { content, status });
  }
  return steps;
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
  sidechain?: boolean,
) => void;

/** Check if a message was already added to avoid duplicates (claude's JSONL
 *  occasionally repeats a message, e.g. around compaction). */
export function isDuplicateMessage(
  session: ClaudeSessionState,
  role: string,
  content: string,
  timestamp?: number,
): boolean {
  if (!content) return false;
  const recent = session.conversation.slice(-5);
  // Key on timestamp when the wire item carried one: a JSONL replay-duplicate
  // re-reads the same transcript line and keeps its *original* timestamp,
  // whereas a genuinely repeated send ('yes', 'continue', …) gets a fresh one.
  // Matching content alone would drop the real repeat — losing a user turn the
  // daemon actually delivered and stranding a phantom optimistic bubble in the
  // renderer (its FIFO expects the user-turn count to grow per delivery). When
  // no timestamp is available we fall back to content-only dedup (unchanged).
  return recent.some(
    (t) =>
      t.role === role &&
      !!t.content &&
      t.content === content &&
      (timestamp === undefined || t.timestamp === timestamp),
  );
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
  // Resolve the discriminant tolerantly (`kind ?? type`), same as the main
  // switch — a marker tagged with `type` would otherwise go undetected and
  // leave the session stuck on 'streaming'.
  const kind = item.kind ?? item.type;
  const text =
    kind === 'user_message' ? item.text : kind === 'tool_result' ? item.content : undefined;
  return typeof text === 'string' && text.trimStart().startsWith('[Request interrupted by user');
}

function tsOf(item: ConversationItemWire): number {
  if (item.timestamp) {
    const ms = Date.parse(item.timestamp);
    if (!Number.isNaN(ms)) return ms;
  }
  return Date.now();
}

/** Edit-shaped tool names (lowercased) across managed providers: codex
 *  apply_patch · opencode/pi edit/write/patch. Same family turnChanges.ts keys
 *  on in the renderer. */
const MANAGED_EDIT_TOOLS = new Set([
  'apply_patch',
  'patch',
  'edit',
  'multiedit',
  'write',
  'notebookedit',
]);

/**
 * Record a managed (non-claude) provider's file edit into `session.fileChanges`.
 * Claude sessions get these from PreToolUse hooks (hookEventRouter); managed
 * providers fire no hooks, so without this their InspectorCard Files tab and
 * fleet-deck file stats stay empty. Runs inside the tool_use dedup (a
 * re-delivered id never reaches here), so entries aren't double-recorded.
 */
function recordManagedFileChange(session: ClaudeSessionState, tc: ToolCall, ts: number): void {
  if (!MANAGED_EDIT_TOOLS.has(tc.name.toLowerCase())) return;
  const paths: string[] = [];
  // Multi-file apply_patch (codex app-server): `changes: [{ path, kind, diff }]`.
  if (Array.isArray(tc.input?.changes)) {
    for (const ch of tc.input.changes) {
      if (typeof ch?.path === 'string' && ch.path) paths.push(ch.path);
    }
  }
  if (paths.length === 0) {
    const single = tc.input?.file_path ?? tc.input?.path ?? tc.input?.filePath;
    if (typeof single === 'string' && single) paths.push(single);
  }
  for (const path of paths) {
    session.fileChanges.push({ path, toolName: tc.name, input: tc.input, timestamp: ts });
  }
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
  // Tool ids already in the timeline. tool_use ids are globally unique
  // (toolu_…), so a re-delivered call — the transcript repeating rows around
  // compaction (same reason isDuplicateMessage exists), a resume replay, an
  // adapter double-emit — must be dropped, not rendered as a duplicate turn.
  // Maintained as we push so in-batch repeats dedupe too, and reused by the
  // hook-reaping housekeeping at the bottom.
  const convToolIds = new Set<string>();
  for (const turn of session.conversation) {
    if (turn.toolCalls) for (const tc of turn.toolCalls) convToolIds.add(tc.id);
  }

  for (const item of items) {
    // The daemon tags the discriminant as `kind`, but tolerate `type` too.
    const kind = item.kind ?? item.type;
    switch (kind) {
      case 'user_message': {
        const text = item.text ?? '';
        // Only discriminate on timestamp when the wire item actually carried
        // one — otherwise tsOf() falls back to Date.now(), which differs per
        // call and would defeat content dedup for timestamp-less replays.
        const ts = item.timestamp ? tsOf(item) : undefined;
        if (text && !isDuplicateMessage(session, 'user', text, ts)) {
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
        // appending snapshots would duplicate.) Claude's PTY transcript path
        // emits whole text blocks and re-emits them around compaction, so it
        // keeps the dedup-and-push path — but Claude's 'stream' transport is a
        // managed adapter emitting per-token deltas, so it must coalesce like
        // the rest or every fragment renders as its own paragraph.
        const streaming =
          session.transport === 'stream' || (!!session.provider && session.provider !== 'claude');
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

      case 'slash_command': {
        const name = item.name ?? '';
        if (!name) break;
        const args = item.args ?? '';
        // On the stream transport the same run arrives twice: the driver
        // echoes it at send time, then the transcript tailer parses the CLI's
        // echo row. Same name+args among the recent turns → it's that pair,
        // not a re-run (mirrors isDuplicateMessage for plain text).
        const recent = session.conversation.slice(-5);
        if (
          recent.some(
            (t) => t.command && t.command.name === name && (t.command.args ?? '') === args,
          )
        )
          break;
        session.conversation.push({
          role: 'user',
          content: `/${name}${args ? ` ${args}` : ''}`,
          timestamp: tsOf(item),
          command: args ? { name, args } : { name },
        });
        break;
      }

      case 'command_output': {
        const output = item.output ?? '';
        if (!output) break;
        // Attach to the nearest preceding command turn that has no output yet
        // (results follow their runs closely — same scan as tool_result).
        let attached = false;
        const floor = Math.max(0, session.conversation.length - 10);
        for (let i = session.conversation.length - 1; i >= floor; i--) {
          const turn = session.conversation[i];
          if (!turn.command) continue;
          if (turn.command.output == null) {
            turn.command.output = output;
            if (item.is_error) turn.command.outputIsError = true;
            attached = true;
          } else if (turn.command.output === output) {
            attached = true; // resync replay of the same output
          }
          break;
        }
        if (!attached) {
          // Output with no visible invocation (e.g. the echo scrolled out of
          // the window) — surface it as a name-less command card.
          session.conversation.push({
            role: 'user',
            content: output,
            timestamp: tsOf(item),
            command: item.is_error
              ? { name: '', output, outputIsError: true }
              : { name: '', output },
          });
        }
        break;
      }

      case 'tool_use': {
        if (item.id && convToolIds.has(item.id)) break;
        const ts = tsOf(item);
        const tc: ToolCall = {
          id: item.id || `tc-${ts}-${Math.random().toString(36).slice(2, 6)}`,
          name: item.name ?? 'unknown',
          input: item.input ?? {},
          status: 'complete',
          startedAt: ts,
          completedAt: ts,
        };
        convToolIds.add(tc.id);
        session.totalToolCalls++;
        // Each tool call is its own turn — interlaced with text in timeline order
        session.conversation.push({
          role: 'assistant',
          content: '',
          timestamp: ts,
          toolCalls: [tc],
        });
        // Managed providers fire no PreToolUse hooks, so their file edits are
        // recorded here off the conversation stream instead (claude keeps the
        // hook path — doing both would double-count).
        if (session.provider && session.provider !== 'claude') {
          recordManagedFileChange(session, tc, ts);
        }
        // Fallback for daemons that don't yet emit a dedicated `plan` item:
        // Claude's TodoWrite call carries the whole checklist in input.todos.
        // Newest write wins either way (a later `plan` item or TodoWrite call
        // replaces this), so both paths can coexist without conflict.
        if (item.name === 'TodoWrite' && Array.isArray(item.input?.todos)) {
          const steps = normalizePlanSteps(item.input.todos as PlanStepWire[]);
          if (steps.length > 0) session.plan = { steps, updatedAt: ts };
        }
        break;
      }

      case 'plan': {
        // Last-write-wins full replacement (may re-arrive on resync).
        const steps = normalizePlanSteps(item.steps);
        session.plan = {
          steps,
          updatedAt: item.updatedAt ?? item.updated_at ?? tsOf(item),
        };
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
            // The call was created with completedAt == startedAt (the tool_use
            // row's own timestamp). The result's timestamp is when the tool
            // actually finished — stamp it so durations render instead of 0s.
            // Only trust an explicit timestamp: falling back to Date.now()
            // here would inflate durations on a resync replaying old turns.
            if (item.timestamp) {
              const doneMs = Date.parse(item.timestamp);
              if (!Number.isNaN(doneMs) && doneMs >= tc.startedAt) tc.completedAt = doneMs;
            }
            break;
          }
        }
        break;
      }

      case 'usage':
        applyUsageFn(
          session,
          item.model ?? null,
          item.usage ?? {},
          item.message_id ?? null,
          item.sidechain === true,
        );
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
    session.completedToolCalls = session.completedToolCalls.filter((tc) => !convToolIds.has(tc.id));
    session.activeToolCalls = session.activeToolCalls.filter((tc) => !convToolIds.has(tc.id));
  }

  // Interrupt detection: if the batch *ends* on an interrupt marker, the turn
  // was cancelled and no Stop hook is coming — fold in the same cleanup Stop
  // would have done (idle, clear pendings + work log). Only the trailing item
  // counts: an interrupt mid-batch is history the session already moved past
  // (e.g. a full resync replaying an old interrupt), and any follow-up prompt
  // flips the state back to 'streaming' via its UserPromptSubmit hook anyway.
  const lastMeaningful = [...items].reverse().find((i) => (i.kind ?? i.type) !== 'usage');
  if (lastMeaningful && isInterruptMarker(lastMeaningful)) {
    applyStopEvent(session);
  }
}
