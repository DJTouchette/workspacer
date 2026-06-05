/**
 * Cross-agent resolution of attention items — the shared resolve path used by
 * the Triage Inbox and (Phase 2) the Fleet Deck card quick-actions.
 *
 * Every action here addresses the session by id (claudeApprove / claudeAnswer /
 * claudeMessage / claudeWrite), so the inbox can resolve ANY agent's request
 * without owning that agent's pane or its MessagePort `write` closure.
 *
 * The semantics deliberately mirror ClaudePane's handleApprovalRespond (~383)
 * and handleAnswer (~490) — including the two race guards that file documents.
 * ClaudePane remains the source of truth; keep these in sync if it changes.
 */

/**
 * Approve or deny a pending tool-permission request.
 *
 * Mirrors ClaudePane.handleApprovalRespond: prefer the daemon's /approve
 * endpoint; on failure fall back to driving the interactive select menu via a
 * raw PTY write — BUT suppress that fallback when a question picker is also
 * pending, since the keystroke would otherwise select option 1 of the picker
 * by accident (PermissionRequest racing with AskUserQuestion's PreToolUse).
 */
export function resolveApproval(
  sessionId: string,
  response: 'yes' | 'no',
  hasPendingQuestion: boolean,
): void {
  window.electronAPI.claudeApprove(sessionId, response).catch((err) => {
    console.warn('[resolveAttention] /approve failed:', err);
    if (hasPendingQuestion) {
      console.warn('[resolveAttention] suppressed keystroke fallback — question picker is active');
      return;
    }
    // sendApproval-equivalent over the by-id PTY write: Enter approves, two
    // downs + Enter denies.
    window.electronAPI.claudeWrite(sessionId, response === 'yes' ? '\r' : '\x1b[B\x1b[B\r');
  });
}

/**
 * Answer an AskUserQuestion picker.
 *
 * Mirrors ClaudePane.handleAnswer: write directly to the PTY rather than the
 * /answer endpoint, which requires mode=Question and can race with concurrent
 * hook events. claude's TUI picker accepts numeric input + Enter like any
 * keystroke.
 */
export function resolveAnswer(
  sessionId: string,
  payload: { option?: number; text?: string; answers?: string[] },
): void {
  if (payload.option !== undefined) {
    window.electronAPI.claudeWrite(sessionId, `${payload.option}\r`);
  } else if (payload.text !== undefined) {
    window.electronAPI.claudeWrite(sessionId, `${payload.text}\r`);
  } else if (payload.answers) {
    for (const ans of payload.answers) window.electronAPI.claudeWrite(sessionId, `${ans}\r`);
  }
}

/**
 * Send a free-text message to an agent (inbox quick-reply).
 *
 * Mirrors ClaudePane.handleSend: prefer claudemon's mode-gated /message
 * endpoint (it appends the carriage return for us); fall back to a raw PTY
 * write + Enter if the session isn't in input mode.
 */
export function resolveReply(sessionId: string, text: string): void {
  window.electronAPI.claudeMessage(sessionId, text).then((res) => {
    if (!res.ok) {
      window.electronAPI.claudeWrite(sessionId, text);
      setTimeout(() => window.electronAPI.claudeWrite(sessionId, '\r'), 50);
    }
  }).catch(() => {
    window.electronAPI.claudeWrite(sessionId, text);
    setTimeout(() => window.electronAPI.claudeWrite(sessionId, '\r'), 50);
  });
}
