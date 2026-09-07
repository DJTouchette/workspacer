import { useCallback, useLayoutEffect, useSyncExternalStore } from 'react';
import type { ConversationTurn } from '../types/claudeSession';

export type ChatSendResult = { ok: boolean; error?: string };
export type SessionChatController = {
  send: (text: string) => Promise<ChatSendResult>;
  conversation: ConversationTurn[];
  pending: (ConversationTurn & { queued?: boolean })[];
};
// A projection of the owning pane, not another transcript/pending-message store.
// A Fleet card borrows the same send handler, optimistic queue and reconciliation
// as expanded chat. Include pane identity so a watch cannot take over the owner.
const controllers = new Map<string, SessionChatController>();
const listeners = new Set<() => void>();
const keyFor = (sessionId: string, paneId: string) => `${sessionId}\0${paneId}`;
const notify = () => listeners.forEach((fn) => fn());
const subscribe = (fn: () => void) => {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
};
export function usePublishSessionChatController(
  sessionId: string | null,
  paneId: string,
  controller: SessionChatController,
) {
  useLayoutEffect(() => {
    if (!sessionId) return;
    controllers.set(keyFor(sessionId, paneId), controller);
    notify();
  }, [sessionId, paneId, controller]);
  useLayoutEffect(() => {
    if (!sessionId) return;
    return () => {
      controllers.delete(keyFor(sessionId, paneId));
      notify();
    };
  }, [sessionId, paneId]);
}
export function useSessionChatController(sessionId?: string, paneId?: string) {
  const snapshot = useCallback(
    () => (sessionId && paneId ? controllers.get(keyFor(sessionId, paneId)) : undefined),
    [sessionId, paneId],
  );
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** Wait for a lazily restored owner; never send via another session or watch. */
export function waitForSessionChatController(
  sessionId: string,
  paneId: () => string | undefined,
  timeoutMs = 5000,
): Promise<SessionChatController | undefined> {
  return new Promise((resolve) => {
    const finish = (value?: SessionChatController) => {
      clearTimeout(timer);
      unsubscribe();
      resolve(value);
    };
    const check = () => {
      const id = paneId();
      const controller = id ? controllers.get(keyFor(sessionId, id)) : undefined;
      if (controller) finish(controller);
    };
    const unsubscribe = subscribe(check);
    const timer = setTimeout(() => finish(), timeoutMs);
    check();
  });
}
