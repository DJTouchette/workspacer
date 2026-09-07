import { useCallback, useRef, useState, type SetStateAction } from 'react';

// Retained data only, never a hidden React viewer. Existing pane lifetimes keep
// their live state; this LRU covers remounts and paged-out inline cards. A large
// fleet cannot leave an unbounded history of closed session drafts behind.
export const MAX_RETAINED_CHAT_SESSIONS = 128;
type Entry = { fields: Map<string, unknown>; alive: boolean };
const sessions = new Map<string, Entry>();
function entryFor(sessionId: string): Entry {
  let entry = sessions.get(sessionId);
  if (!entry) entry = { fields: new Map(), alive: true };
  sessions.delete(sessionId);
  sessions.set(sessionId, entry);
  while (sessions.size > MAX_RETAINED_CHAT_SESSIONS) sessions.delete(sessions.keys().next().value!);
  return entry;
}
export function clearSessionChatUiState(sessionId: string | undefined) {
  if (!sessionId) return;
  const entry = sessions.get(sessionId);
  if (entry) entry.alive = false;
  sessions.delete(sessionId);
}
/** Test isolation: each fixture represents a separate application lifetime. */
export function resetSessionChatUiState() {
  sessions.forEach((entry) => {
    entry.alive = false;
  });
  sessions.clear();
}
export function retainedChatSessionCount() {
  return sessions.size;
}
function initialValue<T>(initial: T | (() => T)): T {
  return typeof initial === 'function' ? (initial as () => T)() : initial;
}
export function useSessionChatState<T>(
  sessionId: string | undefined,
  field: string,
  initial: T | (() => T),
) {
  const slot = useRef<{ key: string; entry: Entry; value: T }>();
  const key = `${sessionId}\0${field}`;
  if (slot.current?.key !== key || !slot.current.entry.alive) {
    const entry = sessionId
      ? entryFor(sessionId)
      : { fields: new Map<string, unknown>(), alive: true };
    const pendingPane =
      slot.current?.key.startsWith('pane:') === true && !sessionId?.startsWith('pane:');
    if (!entry.fields.has(field))
      entry.fields.set(field, pendingPane ? slot.current!.value : initialValue(initial));
    const expired = slot.current?.key === key && !slot.current.entry.alive;
    if (expired && field.startsWith('ref:')) {
      const ref = slot.current!.value as { current: unknown };
      ref.current = (entry.fields.get(field) as { current: unknown }).current;
      entry.fields.set(field, ref);
    }
    if (pendingPane || expired) {
      // Resolve a fresh pane's temporary key without replacing refs/setters that
      // were captured by its mount-time event handlers.
      if (pendingPane) sessions.delete(slot.current!.key.split('\0')[0]);
      Object.assign(slot.current!, { key, entry, value: entry.fields.get(field) as T });
    } else {
      slot.current = { key, entry, value: entry.fields.get(field) as T };
    }
    const inlineKeys = [...entry.fields.keys()].filter((k) => k.startsWith('inline:'));
    for (const old of inlineKeys.slice(0, -256)) entry.fields.delete(old);
  }
  const current = slot.current!;
  const [, render] = useState(0);
  const set = useCallback(
    (next: SetStateAction<T>) => {
      if (!current.entry.alive) return;
      const value = typeof next === 'function' ? (next as (prev: T) => T)(current.value) : next;
      if (Object.is(value, current.value)) return;
      current.value = value;
      current.entry.fields.set(field, value);
      // Bound paged-out inline UI, independently of the number of sessions.
      const inlineKeys = [...current.entry.fields.keys()].filter((k) => k.startsWith('inline:'));
      for (const old of inlineKeys.slice(0, -256)) current.entry.fields.delete(old);
      render((n) => n + 1);
    },
    [current, field],
  );
  return [current.value, set] as const;
}
export function useSessionChatRef<T>(sessionId: string, field: string, initial: T) {
  const [ref] = useSessionChatState(sessionId, `ref:${field}`, () => ({ current: initial }));
  return ref;
}
