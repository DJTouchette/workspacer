import { createContext, useContext } from 'react';
import { useSessionChatState } from '../../hooks/useSessionChatUiState';

export const ChatUiScope = createContext<{ sessionId: string; turn: string } | null>(null);
/** Stable content key: no pane id or React mount id, so paging and pane moves agree. */
function contentKey(value: unknown) {
  const text = JSON.stringify(value) ?? '';
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) hash = Math.imul(hash ^ text.charCodeAt(i), 16777619);
  return `${text.length}:${hash >>> 0}`;
}
export function useInlineChatState<T>(field: string, identity: unknown, initial: T | (() => T)) {
  const scope = useContext(ChatUiScope);
  return useSessionChatState(
    scope?.sessionId,
    `inline:${scope?.turn}:${field}:${contentKey(identity)}`,
    initial,
  );
}
