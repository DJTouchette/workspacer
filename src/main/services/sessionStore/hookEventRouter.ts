import type { ClaudeSessionState, ToolCall } from '../claudeSessionStore';

// ── HookEventRouter ───────────────────────────────────────────────────────────
//
// Pure switch logic: given the current session state + a hook event, mutate the
// session in-place (same as the original inline switch). The caller owns the
// session object and handles side-effects (pushUpdate, watcher, notifier, etc.).

export function applyHookEvent(session: ClaudeSessionState, event: any): void {
  const hookName: string = event.hook_event_name ?? event.type ?? '';

  switch (hookName) {
    case 'SessionStart':
      session.status = 'active';
      session.ambientState = 'idle';
      break;

    case 'UserPromptSubmit':
      session.ambientState = 'streaming';
      break;

    case 'PreToolUse': {
      session.ambientState = 'streaming';
      const tc: ToolCall = {
        id: event.tool_use_id ?? `tc-${Date.now()}`,
        name: event.tool_name ?? 'unknown',
        input: event.tool_input ?? {},
        status: 'running',
        startedAt: Date.now(),
      };
      session.activeToolCalls.push(tc);

      // A new tool call invalidates any stale approval card from a prior
      // tool — the daemon gateway only parks one decision at a time.
      session.pendingApproval = null;

      // AskUserQuestion: surface the question payload as a pending picker.
      // Also defensively clear any stale approval card — these are mutually
      // exclusive: a picker means claude is asking the user, not asking for
      // tool permission.
      if (tc.name === 'AskUserQuestion' && Array.isArray(tc.input?.questions)) {
        session.pendingQuestions = tc.input.questions;
        session.pendingApproval = null;
        session.ambientState = 'waiting_input';
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
      session.ambientState = 'streaming';
      // Any completed tool clears any leftover approval card — the daemon
      // gateway is single-shot, so by the time PostToolUse fires, whatever
      // decision was pending is either resolved or no longer relevant.
      session.pendingApproval = null;
      const completed = session.activeToolCalls.find(t => t.id === event.tool_use_id);
      if (completed) {
        completed.status = 'complete';
        completed.completedAt = Date.now();
        session.activeToolCalls = session.activeToolCalls.filter(t => t.id !== event.tool_use_id);
        session.completedToolCalls.push(completed);
        if (completed.name === 'AskUserQuestion') {
          session.pendingQuestions = null;
        }
      }
      break;
    }

    case 'PostToolUseFailure': {
      const failed = session.activeToolCalls.find(t => t.id === event.tool_use_id);
      if (failed) {
        failed.status = 'failed';
        failed.completedAt = Date.now();
        session.activeToolCalls = session.activeToolCalls.filter(t => t.id !== event.tool_use_id);
        session.completedToolCalls.push(failed);
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
      session.ambientState = 'waiting_approval';
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
      const sub = session.subagents.find(s => s.id === event.agent_id);
      if (sub) {
        sub.status = 'complete';
        sub.completedAt = Date.now();
      }
      break;
    }

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
  session.subagents = session.subagents.filter(s => s.status === 'running');
}

/** Apply the SessionEnd event's synchronous state mutations only. */
export function applySessionEndEvent(session: ClaudeSessionState): void {
  session.status = 'ended';
  session.ambientState = 'idle';
}
