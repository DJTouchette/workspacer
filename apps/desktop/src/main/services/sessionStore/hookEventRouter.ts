import type {
  ClaudeSessionState,
  PendingApproval,
  PendingQuestion,
  ToolCall,
} from '../claudeSessionStore';
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

// ── The pending slot ──────────────────────────────────────────────────────────
//
// INVARIANT: exactly one feed owns a session's pending slot (`pendingApproval`
// + `pendingQuestions`). Other feeds may ENRICH the session, but must never
// park or resolve that slot. Every worker-freeze this project has shipped came
// from breaking it: a card nulled by the feed that did not own it, while the
// owning feed still held the request, leaves a session visibly blocked with
// nothing to answer and no way out but killing it.
//
// Who owns it:
//   • claude on the PTY transport → THIS feed. `PermissionRequest` /
//     `AskUserQuestion` hooks are the only source of its cards, and the
//     matching PostToolUse/Stop are the only thing that clears them.
//   • everything else (any non-claude provider, or claude on the stream
//     transport) → the DAEMON, via its single `pending` slot delivered on the
//     `/events` SSE stream (claudemonEventBridge → applyManagedMode →
//     applyManagedPending). Approvals there are `can_use_tool` control
//     requests the daemon parked; questions are `Pending::Question`
//     (claude_stream.rs parks AskUserQuestion itself), and the daemon resolves
//     both on the turn-closing `result` frame.
//
// The two feeds race: a hook is a `curl` subprocess round-tripping through the
// hook port, `set_managed_mode` is in-process. So the hook feed's view of the
// slot is always possibly-stale, in both directions — it can null a live card
// AND resurrect a dead one.
//
// claudemon is structurally immune to this on its side: `SessionStore::ingest`
// returns early for managed/stream sessions before `SessionState::apply` runs,
// so no hook can reach a driver-owned slot, and since 3064726c a write must
// state its intent as a `PendingWrite` (`Park` / `Resolve` / `Keep`) that the
// store can refuse. This file is the desktop's equivalent gate: the router
// body sees the session through `HookFedSession`, where the pending fields are
// READONLY, so the only way to touch them is `HookPendingSlot` — which knows
// who owns them. A new writer cannot forget the check; it will not compile.

/** Which feed owns this session's pending slot. The mirror of
 *  claudeSessionStore's `daemonOwnsPending` (`provider !== 'claude' ||
 *  transport === 'stream'`), kept spelled the same way round so the two cannot
 *  drift into disagreeing about who owns the card. */
export function pendingSlotOwner(
  session: Pick<ClaudeSessionState, 'provider' | 'transport'>,
): 'hooks' | 'daemon' {
  const isClaude = (session.provider ?? 'claude') === 'claude';
  return isClaude && session.transport !== 'stream' ? 'hooks' : 'daemon';
}

/** A session as the hook feed may mutate it: everything writable EXCEPT the
 *  pending slot, which is readonly here so the compiler routes every write
 *  through {@link HookPendingSlot}. (A mutable object is assignable to this —
 *  `readonly` is not checked in assignability — so callers pass a plain
 *  `ClaudeSessionState`; only this file's body is constrained.) */
type HookFedSession = Omit<ClaudeSessionState, 'pendingApproval' | 'pendingQuestions'> & {
  readonly pendingApproval: ClaudeSessionState['pendingApproval'];
  readonly pendingQuestions: ClaudeSessionState['pendingQuestions'];
};

/**
 * The hook feed's handle on the pending slot, mirroring claudemon's
 * `PendingWrite` intent: a write either PARKS a decision or RESOLVES one, and
 * "keep it as it is" is expressed by not calling at all. Every write is
 * suppressed unless this feed owns the slot.
 *
 * Each method returns what the slot HOLDS after the call — like
 * `set_managed_mode`, so a caller mirrors the real state rather than assuming
 * its own write landed.
 */
class HookPendingSlot {
  private readonly owned: boolean;

  constructor(private readonly session: ClaudeSessionState) {
    this.owned = pendingSlotOwner(session) === 'hooks';
  }

  parkApproval(next: PendingApproval): PendingApproval | null {
    if (this.owned) this.session.pendingApproval = next;
    return this.session.pendingApproval;
  }

  parkQuestions(next: PendingQuestion[]): PendingQuestion[] | null {
    if (this.owned) this.session.pendingQuestions = next;
    return this.session.pendingQuestions;
  }

  resolveApproval(): PendingApproval | null {
    if (this.owned) this.session.pendingApproval = null;
    return this.session.pendingApproval;
  }

  resolveQuestions(): PendingQuestion[] | null {
    if (this.owned) this.session.pendingQuestions = null;
    return this.session.pendingQuestions;
  }

  /** Turn-boundary sweep: both halves at once. Still ownership-gated — for a
   *  daemon-owned session the hook `Stop` is not the owner's turn-end signal
   *  (the driver's own `result` frame is, and it resolves the slot then), and
   *  a `Stop` that lands after the driver parked a background subagent's
   *  request would strand exactly as a late PostToolUse did. */
  resolveAll(): void {
    this.resolveApproval();
    this.resolveQuestions();
  }
}

export function applyHookEvent(session: ClaudeSessionState, event: any): void {
  routeHookEvent(session, new HookPendingSlot(session), event);
}

function routeHookEvent(session: HookFedSession, pending: HookPendingSlot, event: any): void {
  const hookName: string = event.hook_event_name ?? event.type ?? '';

  // Stream-transport Claude sessions (headless stream-json, managed adapter)
  // still fire hooks, but their working/idle/waiting state is owned by the
  // daemon's managed mode stream (`set_managed_mode` → applyManagedMode) — the
  // same channel codex/opencode/pi use. Hooks are ENRICHMENT-ONLY for them:
  // tool cards, file changes, subagents and permission-mode telemetry still
  // apply, but ambientState must not be written here or the two state machines
  // fight (the PTY-era hazard that motivated the stream transport in the first
  // place). The approval/question CARDS are the same split, one level down —
  // see the pending-slot invariant above; `pending` is the only way to reach
  // them from here.
  const hooksOwnAmbient = session.transport !== 'stream';
  const setAmbient = (state: ClaudeSessionState['ambientState']): void => {
    if (hooksOwnAmbient) session.ambientState = state;
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
      pending.resolveApproval();

      // AskUserQuestion: surface the question payload as a pending picker.
      // Also defensively clear any stale approval card — these are mutually
      // exclusive: a picker means claude is asking the user, not asking for
      // tool permission.
      if (tc.name === 'AskUserQuestion' && Array.isArray(tc.input?.questions)) {
        // The raw tool input spells it `multiSelect`; our types (and the
        // picker) read snake_case, so normalize at the ingest seam.
        pending.parkQuestions(
          tc.input.questions.map((q: { multi_select?: boolean; multiSelect?: boolean }) => ({
            ...q,
            multi_select: q.multi_select ?? q.multiSelect ?? false,
          })),
        );
        pending.resolveApproval();
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
      pending.resolveApproval();
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
          // Same ownership caveat again: on a stream session the picker was
          // parked by the driver (`Pending::Question`), and this hook is the
          // slow feed — it must not answer-by-forgetting a question the driver
          // is still holding open.
          pending.resolveQuestions();
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
      // Ownership-gated like every other write to the slot. A daemon-owned
      // session gets its card from the driver's parked `can_use_tool`, and
      // this feed adding one is not harmless: arriving late it resurrects an
      // approval the daemon already resolved, and it surfaces queued
      // non-head requests the daemon deliberately hides (claude_stream.rs
      // parks one at a time and re-surfaces the next only on an answer).
      pending.parkApproval({
        toolName: event.tool_name ?? '',
        toolInput: event.tool_input ?? {},
        suggestions: event.permission_suggestions,
        timestamp: Date.now(),
      });
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
  applyStopEventTo(session, new HookPendingSlot(session));
}

function applyStopEventTo(session: HookFedSession, pending: HookPendingSlot): void {
  // Stream-transport sessions own their working/idle state via the daemon's
  // managed mode (set_managed_mode → applyManagedMode) — the stream driver
  // already holds the turn busy while a background subagent runs
  // (`bg_tasks_active` / `suppress_idle`). The daemon still rebroadcasts the raw
  // Stop hook to us, so writing ambientState here would clobber that back to
  // idle mid-subagent. Leave it untouched — hooks are enrichment-only for
  // stream (same invariant as routeHookEvent's `hooksOwnAmbient`).
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
  // A turn boundary sweeps the cards — but only the cards this feed owns. For
  // a daemon-owned session the driver's own turn end is the `result` frame,
  // which clears its parked requests and resolves the slot
  // (`AgentUpdate::Idle` → `PendingWrite::Resolve`); this Stop hook is the
  // slow, second feed. A background subagent that parks a `can_use_tool`
  // before the parent's Stop lands would be stranded here exactly as a late
  // PostToolUse stranded one — blocked state, no card.
  pending.resolveAll();
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
