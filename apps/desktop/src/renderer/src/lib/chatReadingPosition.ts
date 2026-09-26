import type { ConversationTurn } from '../types/claudeSession';

export const CHAT_READING_STORAGE = 'workspacer:chat-reading:v1';
export interface ReadThrough {
  index: number;
  timestamp: number;
  role: string;
  fingerprint: string;
}
export interface ChatReadingPosition {
  anchor: string;
  offset: number;
  atBottom: boolean;
  readThrough?: ReadThrough;
  updatedAt: number;
}

/** Store coordinates and hashes only, never transcript text. */
export function turnFingerprint(turn: ConversationTurn): string {
  const value = JSON.stringify([
    turn.content,
    turn.command,
    turn.toolCalls?.map((call) => [call.id, call.status, call.completedAt]),
  ]);
  let hash = 2166136261;
  for (let i = 0; i < value.length; i++) hash = Math.imul(hash ^ value.charCodeAt(i), 16777619);
  return (hash >>> 0).toString(36);
}

export function readThrough(turns: ConversationTurn[], offset: number): ReadThrough | undefined {
  const last = turns.at(-1);
  return last
    ? {
        index: offset + turns.length - 1,
        timestamp: last.timestamp,
        role: last.role,
        fingerprint: turnFingerprint(last),
      }
    : undefined;
}

export function firstUnreadTurn(
  seen: ReadThrough | undefined,
  turns: ConversationTurn[],
  offset: number,
): number | null {
  if (!seen || !turns.length || seen.index >= offset + turns.length) return null;
  if (seen.index < offset) return offset;
  const previous = turns[seen.index - offset];
  // A rewritten/replaced transcript is not the old coordinate space.
  if (previous.timestamp !== seen.timestamp || previous.role !== seen.role) return null;
  if (turnFingerprint(previous) !== seen.fingerprint) return seen.index;
  return seen.index + 1 < offset + turns.length ? seen.index + 1 : null;
}

function positions(): Record<string, ChatReadingPosition> {
  try {
    const raw = JSON.parse(localStorage.getItem(CHAT_READING_STORAGE) ?? '{}');
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {};
    return Object.fromEntries(
      Object.entries(raw).filter(
        ([, v]: [string, any]) =>
          v &&
          typeof v.anchor === 'string' &&
          /^(msg|work|chg)-\d+$/.test(v.anchor) &&
          Number.isFinite(v.offset) &&
          typeof v.atBottom === 'boolean' &&
          Number.isFinite(v.updatedAt) &&
          Date.now() - v.updatedAt < 30 * 86400000 &&
          (!v.readThrough ||
            (Number.isInteger(v.readThrough.index) &&
              v.readThrough.index >= 0 &&
              Number.isFinite(v.readThrough.timestamp) &&
              typeof v.readThrough.role === 'string' &&
              typeof v.readThrough.fingerprint === 'string')),
      ),
    ) as Record<string, ChatReadingPosition>;
  } catch {
    return {};
  }
}

export function loadChatReadingPosition(key: string): ChatReadingPosition | undefined {
  return positions()[key];
}

export function saveChatReadingPosition(key: string, position: ChatReadingPosition): void {
  try {
    const entries = Object.entries({ ...positions(), [key]: position })
      .sort((a, b) => b[1].updatedAt - a[1].updatedAt)
      .slice(0, 128);
    localStorage.setItem(CHAT_READING_STORAGE, JSON.stringify(Object.fromEntries(entries)));
  } catch {
    /* Storage may be disabled or full; reading must still work. */
  }
}
