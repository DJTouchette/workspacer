/**
 * chatScrollBus — keyboard-driven scrolling for the ACTIVE Claude pane's chat.
 *
 * The command layer's chat verbs (prefix Shift+K/J half-page, g g top,
 * Shift+G bottom) fire in the window dispatcher, which knows nothing about
 * pane internals; the active ClaudePane subscribes and applies the motion to
 * its own scroll container (whose sticky-bottom anchoring must stay in that
 * pane's hands). Same tiny pattern as reviewBus: module-level listener set,
 * no React context, safe to call from native event handlers.
 */

export type ChatScrollKind = 'half-up' | 'half-down' | 'top' | 'bottom';

type Listener = (kind: ChatScrollKind) => void;

const listeners = new Set<Listener>();

export function onChatScroll(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function requestChatScroll(kind: ChatScrollKind): void {
  for (const l of [...listeners]) l(kind);
}
