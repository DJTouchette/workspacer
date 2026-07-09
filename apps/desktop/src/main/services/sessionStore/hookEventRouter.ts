import type { ClaudeSessionState, ToolCall } from '../claudeSessionStore';

// ── HookEventRouter ───────────────────────────────────────────────────────────
//
// Pure switch logic: given the current session state + a hook event, mutate the
// session in-place (same as the original inline switch). The caller owns the
// session object and handles side-effects (pushUpdate, watcher, notifier, etc.).

// Safety caps so the live work log can't grow without bound if Stop (which
// clears these) is never delivered — e.g. a session that dies mid-turn, or a
// tool whose id never matches the transcript so housekeeping can't reap it.
// Normal turns sit far below these; they only bound pathological cases.
const MAX_ACTIVE_TOOL_CALLS = 50;
const MAX_COMPLETED_TOOL_CALLS = 50;

/** Keep only the most recent `max` entries, mutating in place. */
function capInPlace<T>(arr: T[], max: number): void {
  if (arr.length > max) arr.splice(0, arr.length - max);
}

/** Whether a PostToolUse `tool_response` payload signals a tool failure.
 *  Claude wraps errors as `{is_error:true}`; some tools return a plain string. */
function toolResponseIsError(resp: unknown): boolean {
  if (!resp) return false;
  if (typeof resp === 'object') {
    const r = resp as { is_error?: unknown; isError?: unknown };
    return r.is_error === true || r.isError === true;
  }
  return false;
}

export function applyHookEvent(session: ClaudeSessionState, event: any): void {
  const hookName: string = event.hook_event_name ?? event.type ?? '';

  // Stream-transport Claude sessions (headless stream-json, managed adapter)
  // still fire hooks, but their working/idle/waiting state is owned by the
  // daemon's managed mode stream (`set_managed_mode` → applyManagedMode) — the
  // same channel codex/opencode/pi use. Hooks are ENRICHMENT-ONLY for them:
  // tool cards, file changes, subagents, approval/question payloads and
  // permission-mode telemetry still apply, but ambientState must not be
  // written here or the two state machines fight (the PTY-era hazard that
  // motivated the stream transport in the first place).
  const hooksOwnAmbient = session.transport !== 'stream';
  const setAmbient = (state: ClaudeSessionState['ambientState']): void => {
    if (hooksOwnAmbient) session.ambientState = state;
  };

  switch (hookName) {
    case 'SessionStart':
      session.status = 'active';
      setAmbient('idle');
      break;

    case 'UserPromptSubmit':
      setAmbient('streaming');
      break;

    case 'PreToolUse': {
      setAmbient('streaming');

      // Tool calls executed inside a subagent carry agent_id (verified on CLI
      // 2.1.201: the subagent's Bash hook has agent_id/agent_type, the parent's
      // own calls don't). They belong to the subagent's transcript and watch
      // pane — not the main chat's live work log — and a parallel subagent
      // tool must not clear the parent's pending approval/question cards.
      // Its file edits are still real changes, so those are recorded.
      if (event.agent_id) {
        if (['Edit', 'MultiEdit', 'Write'].includes(event.tool_name)) {
          session.fileChanges.push({
            path: event.tool_input?.file_path ?? 'unknown',
            toolName: event.tool_name,
            input: event.tool_input ?? {},
            timestamp: Date.now(),
          });
        }
        break;
      }

      const id: string =
        event.tool_use_id ?? `tc-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;

      // Idempotent on tool_use_id: a re-delivered PreToolUse (e.g. after an SSE
      // reconnect) must not spawn a second card for a tool we already track —
      // that's a prime way active calls "pile up at the bottom". Skip the whole
      // block so file changes aren't double-recorded either.
      if (
        session.activeToolCalls.some((t) => t.id === id) ||
        session.completedToolCalls.some((t) => t.id === id)
      ) {
        break;
      }

      const tc: ToolCall = {
        id,
        name: event.tool_name ?? 'unknown',
        input: event.tool_input ?? {},
        status: 'running',
        startedAt: Date.now(),
      };
      session.activeToolCalls.push(tc);
      capInPlace(session.activeToolCalls, MAX_ACTIVE_TOOL_CALLS);

      // A new tool call invalidates any stale approval card from a prior
      // tool — the daemon gateway only parks one decision at a time.
      session.pendingApproval = null;

      // AskUserQuestion: surface the question payload as a pending picker.
      // Also defensively clear any stale approval card — these are mutually
      // exclusive: a picker means claude is asking the user, not asking for
      // tool permission.
      if (tc.name === 'AskUserQuestion' && Array.isArray(tc.input?.questions)) {
        // The raw tool input spells it `multiSelect`; our types (and the
        // picker) read snake_case, so normalize at the ingest seam.
        session.pendingQuestions = tc.input.questions.map(
          (q: { multi_select?: boolean; multiSelect?: boolean }) => ({
            ...q,
            multi_select: q.multi_select ?? q.multiSelect ?? false,
          }),
        );
        session.pendingApproval = null;
        setAmbient('waiting_input');
      }

      if (['Edit', 'MultiEdit', 'Write'].includes(tc.name)) {
        session.fileChanges.push({
          path: tc.input?.file_path ?? 'unknown',
          toolName: tc.name,
          input: tc.input,
          timestamp: Date.now(),
        });
      }
      break;
    }

    case 'PostToolUse': {
      setAmbient('streaming');
      // Subagent tool completions aren't tracked here (see PreToolUse) and
      // must not clear the parent's pending cards.
      if (event.agent_id) break;
      // Any completed tool clears any leftover approval card — the daemon
      // gateway is single-shot, so by the time PostToolUse fires, whatever
      // decision was pending is either resolved or no longer relevant.
      session.pendingApproval = null;
      const completed = session.activeToolCalls.find((t) => t.id === event.tool_use_id);
      if (completed) {
        // Claude's real PostToolUse fires on success AND failure; the tool's
        // error shows up in `tool_response` (either `{is_error:true}` or a
        // string that reads as an error). Mark failed so the trace work-log
        // shows it as such rather than a normal completion.
        completed.status = toolResponseIsError(event.tool_response) ? 'failed' : 'complete';
        completed.completedAt = Date.now();
        session.activeToolCalls = session.activeToolCalls.filter((t) => t.id !== event.tool_use_id);
        session.completedToolCalls.push(completed);
        capInPlace(session.completedToolCalls, MAX_COMPLETED_TOOL_CALLS);
        if (completed.name === 'AskUserQuestion') {
          session.pendingQuestions = null;
        }
      }
      break;
    }

    case 'PostToolUseFailure': {
      const failed = session.activeToolCalls.find((t) => t.id === event.tool_use_id);
      if (failed) {
        failed.status = 'failed';
        failed.completedAt = Date.now();
        session.activeToolCalls = session.activeToolCalls.filter((t) => t.id !== event.tool_use_id);
        session.completedToolCalls.push(failed);
        capInPlace(session.completedToolCalls, MAX_COMPLETED_TOOL_CALLS);
      }
      break;
    }

    case 'PermissionRequest':
      session.pendingApproval = {
        toolName: event.tool_name ?? '',
        toolInput: event.tool_input ?? {},
        suggestions: event.permission_suggestions,
        timestamp: Date.now(),
      };
      setAmbient('waiting_approval');
      break;

    case 'SubagentStart':
      session.subagents.push({
        id: event.agent_id ?? `sa-${Date.now()}`,
        type: event.agent_type ?? 'unknown',
        status: 'running',
        startedAt: Date.now(),
      });
      break;

    case 'SubagentStop': {
      const sub = session.subagents.find((s) => s.id === event.agent_id);
      if (sub) {
        sub.status = 'complete';
        sub.completedAt = Date.now();
      }
      break;
    }

    // Context compaction brackets (enrichment-only, like tool cards — safe for
    // stream sessions too since they don't touch ambientState).
    case 'PreCompact':
      session.compacting = true;
      break;

    case 'PostCompact':
      session.compacting = false;
      session.lastCompactAt = Date.now();
      session.compactionCount = (session.compactionCount ?? 0) + 1;
      break;

    case 'Notification':
      session.conversation.push({
        role: 'assistant',
        content: event.message ?? event.notification ?? '[notification]',
        timestamp: Date.now(),
      });
      break;

    // Note: 'Stop' and 'SessionEnd' are handled by the coordinator because
    // they require access to store-level side-effects (setTimeout refresh,
    // workflowWatcher.detach, forgetTelemetry, writeHistory). Only the
    // in-place state mutations are factored here; the coordinator delegates
    // those two cases after calling applyHookEvent for all other state.
    default:
      break;
  }
}

/** Apply the Stop event's synchronous state mutations only. */
export function applyStopEvent(session: ClaudeSessionState): void {
  session.ambientState = 'idle';
  session.pendingApproval = null;
  session.pendingQuestions = null;
  // Clear tool calls — they're already shown inline in conversation via transcript
  session.activeToolCalls = [];
  session.completedToolCalls = [];
  session.subagents = session.subagents.filter((s) => s.status === 'running');
}

/** Apply the SessionEnd event's synchronous state mutations only. */
export function applySessionEndEvent(session: ClaudeSessionState): void {
  session.status = 'ended';
  session.ambientState = 'idle';
}
