import type { ClaudeSessionState, ToolCall } from '../claudeSessionStore';
import {
  capConversationInPlace,
  capInPlace,
  MAX_ACTIVE_TOOL_CALLS,
  MAX_COMPLETED_TOOL_CALLS,
  MAX_FILE_CHANGES,
  truncateToolInput,
} from './bounds';

// ── HookEventRouter ───────────────────────────────────────────────────────────
//
// Pure switch logic: given the current session state + a hook event, mutate the
// session in-place (same as the original inline switch). The caller owns the
// session object and handles side-effects (pushUpdate, watcher, notifier, etc.).

// Safety caps live in ./bounds so this file and conversationApplier — the two
// ingest paths into a session — can't drift. Normal turns sit far below them;
// they bound pathological cases, and (since every flush clones the whole
// session to the renderer) the per-frame IPC cost with them.

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

  // ...and the same split applies to the approval CARD, which is what a
  // blocked session is answered through. For a daemon-owned session
  // (`daemonOwnsPending` in claudeSessionStore: any non-claude provider, or
  // claude on the stream transport) the approval never came from a hook at
  // all — it is a `can_use_tool` control request the daemon parked, delivered
  // over the SEPARATE `/events` SSE connection. The hook feed
  // (`/hooks/stream`) is a second, slower connection with no ordering
  // guarantee against it: the CLI's PreToolUse/PostToolUse hook is a `curl`
  // subprocess round-tripping through the hook port, while `set_managed_mode`
  // is in-process.
  //
  // So a hook clearing `pendingApproval` here could — and did — null the card
  // for an approval the daemon is still holding, while `ambientState` stayed
  // `waiting_approval` because of the guard above. That is precisely the
  // unresolvable block reported on 2026-08-22: blocked forever, visibly
  // waiting, with nothing to approve and no way out but killing the session.
  // Never clear what this feed does not own; the daemon's own `pending` slot
  // (applyManagedPending) clears it when the decision is really resolved.
  //
  // The mirror of claudeSessionStore's `daemonOwnsPending`
  // (`provider !== 'claude' || transport === 'stream'`) — kept spelled the same
  // way round so the two cannot drift into disagreeing about who owns the card.
  //
  // Scope: this guards the MID-TURN clears only (PreToolUse / PostToolUse).
  // `PermissionRequest` still writes the card on every transport — it can only
  // ever ADD one, so it cannot strand a session — and `applyStopEvent` still
  // sweeps it, because a turn boundary genuinely means nothing can still be
  // parked. It is "one tool ended, so the other feed's card must be stale"
  // that is the false inference.
  const hooksOwnPending = (session.provider ?? 'claude') === 'claude' && hooksOwnAmbient;
  const clearPendingApproval = (): void => {
    if (hooksOwnPending) session.pendingApproval = null;
  };

  switch (hookName) {
    case 'SessionStart':
      session.status = 'active';
      session.parentTurnEnded = false;
      setAmbient('idle');
      break;

    case 'UserPromptSubmit':
      // A fresh user turn supersedes any prior turn's background subagent work.
      session.parentTurnEnded = false;
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
            input: truncateToolInput(event.tool_input ?? {}),
            timestamp: Date.now(),
          });
          capInPlace(session.fileChanges, MAX_FILE_CHANGES);
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
        // A Write/Edit hook payload carries the whole file body. Bounded here,
        // once, so it can't be re-cloned to the renderer at flush rate for the
        // rest of the session — the identifying fields below are short and
        // survive verbatim (see truncateToolInput).
        input: truncateToolInput(event.tool_input ?? {}),
        status: 'running',
        startedAt: Date.now(),
      };
      session.activeToolCalls.push(tc);
      capInPlace(session.activeToolCalls, MAX_ACTIVE_TOOL_CALLS);

      // A new tool call invalidates any stale approval card from a prior
      // tool — the daemon gateway only parks one decision at a time. Only for
      // a card this feed owns: on a daemon-owned session this hook is the
      // laggy half of the race described above, and the "stale" card it wants
      // to drop is routinely the LIVE one the daemon just parked.
      clearPendingApproval();

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
        clearPendingApproval();
        setAmbient('waiting_input');
      }

      if (['Edit', 'MultiEdit', 'Write'].includes(tc.name)) {
        session.fileChanges.push({
          path: tc.input?.file_path ?? 'unknown',
          toolName: tc.name,
          input: tc.input,
          timestamp: Date.now(),
        });
        capInPlace(session.fileChanges, MAX_FILE_CHANGES);
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
      // decision was pending is either resolved or no longer relevant. Same
      // ownership caveat as PreToolUse: "by the time PostToolUse fires" only
      // holds within ONE feed, and the daemon's approvals arrive on another.
      clearPendingApproval();
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

    case 'SubagentStart': {
      const saId = event.agent_id ?? `sa-${Date.now()}`;
      // Idempotent on agent_id: a re-delivered SubagentStart (a double-fired /
      // retried hook POST) must not spawn a second subagent row — the duplicate
      // inflates the watch pane and analytics subagentCount, and only one copy
      // gets marked complete by the single matching SubagentStop, leaving a
      // phantom 'running' subagent that pins the parent on 'background' forever
      // (mirrors the PreToolUse idempotency guard on tool_use_id above).
      if (event.agent_id && session.subagents.some((s) => s.id === saId)) {
        break;
      }
      session.subagents.push({
        id: saId,
        type: event.agent_type ?? 'unknown',
        status: 'running',
        startedAt: Date.now(),
      });
      break;
    }

    case 'SubagentStop': {
      const sub = session.subagents.find((s) => s.id === event.agent_id);
      if (sub) {
        sub.status = 'complete';
        sub.completedAt = Date.now();
      }
      // If the parent's own turn already ended (its Stop fired while this
      // subagent kept running in the background) and this was the last running
      // subagent, the real idle rides in now — see applyStopEvent below.
      if (session.parentTurnEnded && !session.subagents.some((s) => s.status === 'running')) {
        session.parentTurnEnded = false;
        setAmbient('idle');
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
      // The other ingest path caps per batch; this one pushes a turn at a time.
      capConversationInPlace(session);
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

/** True when work the agent spawned is still running after its own turn:
 *  an async background subagent, or a Workflow run (which detaches from the
 *  turn immediately — its parent Stop fires while the workflow grinds on). */
export function sessionHasBackgroundWork(session: ClaudeSessionState): boolean {
  return (
    session.subagents.some((s) => s.status === 'running') ||
    session.workflows.some((w) => w.status === 'running')
  );
}

/**
 * Keep 'idle' honest: a session whose own turn ended but whose spawned work
 * (workflow / background subagent) is still running shows 'background', and
 * drops back to 'idle' only when that work finishes. Call after anything that
 * can change ambientState or the workflow/subagent sets — and BEFORE the
 * notifier reads the transition, so "finished" fires on true idle only.
 * Ended sessions are left alone.
 */
export function normalizeBackgroundAmbient(session: ClaudeSessionState): void {
  if (session.status === 'ended') return;
  if (session.ambientState === 'idle' && sessionHasBackgroundWork(session)) {
    session.ambientState = 'background';
  } else if (session.ambientState === 'background' && !sessionHasBackgroundWork(session)) {
    session.ambientState = 'idle';
  }
}

/** Apply the Stop event's synchronous state mutations only. */
export function applyStopEvent(session: ClaudeSessionState): void {
  // Stream-transport sessions own their working/idle state via the daemon's
  // managed mode (set_managed_mode → applyManagedMode) — the stream driver
  // already holds the turn busy while a background subagent runs
  // (`bg_tasks_active` / `suppress_idle`). The daemon still rebroadcasts the raw
  // Stop hook to us, so writing ambientState here would clobber that back to
  // idle mid-subagent. Leave it untouched — hooks are enrichment-only for
  // stream (same invariant as applyHookEvent's `hooksOwnAmbient`).
  const hooksOwnAmbient = session.transport !== 'stream';
  if (hooksOwnAmbient) {
    // PTY/hook path: a background (async Agent/Task) subagent can still be
    // running when the parent's own turn ends — its Stop fires while the
    // subagent works on. Hold 'background' and let the last SubagentStop ride
    // the real idle in (see the SubagentStop case above); otherwise idle now
    // (normalizeBackgroundAmbient re-raises it if a workflow is still going).
    const bgSubagentRunning = session.subagents.some((s) => s.status === 'running');
    session.ambientState = bgSubagentRunning ? 'background' : 'idle';
    session.parentTurnEnded = bgSubagentRunning;
  }
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
  session.parentTurnEnded = false;
}
