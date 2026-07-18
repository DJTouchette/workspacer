/**
 * Cross-agent resolution of attention items — the shared resolve path used by
 * the Triage Inbox.
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
  response: 'yes' | 'no' | 'always',
  hasPendingQuestion: boolean,
  provider?: string,
): void {
  window.electronAPI.claudeApprove(sessionId, response).catch((err) => {
    console.warn('[resolveAttention] /approve failed:', err);
    if (hasPendingQuestion) {
      console.warn('[resolveAttention] suppressed keystroke fallback — question picker is active');
      return;
    }
    // The keystrokes encode Claude's 3-row permission menu; a managed
    // provider's PTY (codex/opencode/pi) has a different approval UI, so the
    // daemon endpoint is their only path.
    if (provider && provider !== 'claude') return;
    // sendApproval-equivalent over the by-id PTY write, matching claude's 3-row
    // permission menu: Enter approves (row 1), one down approves-for-session
    // ("allow all", row 2), two downs deny (row 3).
    const keys = response === 'yes' ? '\r' : response === 'always' ? '\x1b[B\r' : '\x1b[B\x1b[B\r';
    window.electronAPI.claudeWrite(sessionId, keys);
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
  payload: { option?: number; text?: string; answers?: string[]; answerKinds?: string[] },
  provider?: string,
): void {
  // Non-claude questions are the daemon's parked AskUserQuestion MCP call —
  // only POST /answer resolves them; the provider's own TUI (if any) knows
  // nothing about the picker, so keystrokes would be garbage input.
  if (provider && provider !== 'claude') {
    window.electronAPI.claudeAnswer(sessionId, payload).catch((err) => {
      console.warn('[resolveAttention] /answer failed (no PTY fallback exists):', err);
    });
    return;
  }
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
 * Mirrors ClaudePane.handleSend: claudemon's /message owns delivery — it
 * queues while the agent is busy (or a dialog is up), injects once the prompt
 * settles, and verifies the submit took. A rejection means the session has
 * ended, where raw keystrokes can't help either; the raw write stays reserved
 * for transport failure (daemon unreachable), framed as a bracketed paste +
 * separate Enter so the CR can't fold into the paste (mirrors the daemon's
 * send_message_now).
 */
export function resolveReply(sessionId: string, text: string): void {
  window.electronAPI
    .claudeMessage(sessionId, text)
    .then((res) => {
      if (!res.ok) {
        console.warn(
          `[resolveAttention] /message rejected (mode=${res.mode}); session is not accepting input`,
        );
      }
    })
    .catch(() => {
      window.electronAPI.claudeWrite(
        sessionId,
        '\x1b[200~' + text.replace(/[\r\n]+$/, '') + '\x1b[201~\r',
      );
    });
}
