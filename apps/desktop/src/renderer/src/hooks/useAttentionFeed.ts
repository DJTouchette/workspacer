import { useEffect, useMemo, useRef, useState, useCallback } from 'react';
import type { AgentWorkspace } from '../types/pane';
import type { AttentionItem, AttentionKind } from '../types/attention';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import { KIND_PRIORITY, sortItems } from '../lib/attentionRouter';

/** Stable string hash (djb2) for building idempotent item signatures. */
function hash(s: string): string {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
}

/** First human-meaningful string out of a tool input, for an approval preview. */
function toolInputPreview(input: any): string {
  if (!input || typeof input !== 'object') return '';
  const v = input.command ?? input.file_path ?? input.path ?? input.pattern ?? input.url
    ?? Object.values(input).find((x) => typeof x === 'string');
  return typeof v === 'string' ? v.split('\n')[0].slice(0, 80) : '';
}

/** Last assistant message text, for the "Finished" card body. */
function lastAssistant(snap: ClaudeSessionSnapshot): string {
  const turns = snap.conversation ?? [];
  for (let i = turns.length - 1; i >= 0; i--) {
    if (turns[i].role === 'assistant' && turns[i].content?.trim()) {
      return turns[i].content.trim().slice(0, 160);
    }
  }
  return '';
}

export interface AttentionFeed {
  items: AttentionItem[];
  counts: { total: number; needsYou: number; byKind: Record<AttentionKind, number> };
  dismiss: (signature: string) => void;
  snooze: (signature: string, minutes: number) => void;
}

export interface AttentionFeedOptions {
  /** Per-kind enable flags (noise control). Defaults all-on. */
  enabledKinds?: Partial<Record<AttentionKind, boolean>>;
}

/**
 * Derive the cross-agent attention feed from the promoted snapshot store.
 *
 * Items live entirely in the renderer (MVP): approval ← pendingApproval,
 * question ← pendingQuestions, done ← a working→idle transition. Re-arriving
 * snapshots update items in place (dedup by signature), never duplicate. When
 * an agent resolves a request, its next snapshot simply drops the item.
 */
export function useAttentionFeed(
  snapshotBySession: Record<string, ClaudeSessionSnapshot>,
  agents: AgentWorkspace[],
  opts: AttentionFeedOptions = {},
): AttentionFeed {
  const enabled = (k: AttentionKind) => opts.enabledKinds?.[k] !== false;

  const [dismissed, setDismissed] = useState<Set<string>>(() => new Set());
  const [snoozedUntil, setSnoozedUntil] = useState<Map<string, number>>(() => new Map());
  const [now, setNow] = useState(() => Date.now());

  // working→idle "done" detection needs memory of each session's prior state.
  const prevStateRef = useRef<Map<string, string>>(new Map());
  const doneAtRef = useRef<Map<string, number>>(new Map());
  const [doneTick, setDoneTick] = useState(0);

  useEffect(() => {
    let changed = false;
    for (const sid of Object.keys(snapshotBySession)) {
      const cur = snapshotBySession[sid]?.ambientState;
      if (!cur) continue;
      const prev = prevStateRef.current.get(sid);
      if (prev === cur) continue;
      const wasWorking = prev === 'thinking' || prev === 'streaming';
      if (cur === 'idle' && wasWorking) {
        doneAtRef.current.set(sid, snapshotBySession[sid].lastActivity || Date.now());
        changed = true;
      } else if (cur === 'thinking' || cur === 'streaming' || cur === 'waiting_input' || cur === 'waiting_approval') {
        if (doneAtRef.current.delete(sid)) changed = true;
      }
      prevStateRef.current.set(sid, cur);
    }
    if (changed) setDoneTick((t) => t + 1);
  }, [snapshotBySession]);

  // Snooze-expiry ticker (also re-surfaces snoozed items when their time is up).
  useEffect(() => {
    const i = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(i);
  }, []);

  const items = useMemo(() => {
    const out: AttentionItem[] = [];
    for (const agent of agents) {
      if (agent.global || !agent.sessionId) continue;
      const snap = snapshotBySession[agent.sessionId];
      if (!snap) continue;
      const sid = agent.sessionId;
      const base = { agentId: agent.id, agentName: agent.name, sessionId: sid, cwd: agent.cwd };

      if (enabled('approval') && snap.pendingApproval) {
        const ap = snap.pendingApproval;
        const sig = `${sid}:approval:${hash(ap.toolName + JSON.stringify(ap.toolInput ?? {}))}`;
        out.push({
          ...base, id: sig, signature: sig, kind: 'approval', priority: KIND_PRIORITY.approval,
          createdAt: ap.timestamp || Date.now(), status: 'open',
          title: ap.toolName, detail: toolInputPreview(ap.toolInput),
          payload: { type: 'approval', approval: ap },
        });
      }

      if (enabled('question') && snap.pendingQuestions?.length) {
        const qs = snap.pendingQuestions;
        const sig = `${sid}:question:${hash(qs.map((q) => q.question).join('|'))}`;
        out.push({
          ...base, id: sig, signature: sig, kind: 'question', priority: KIND_PRIORITY.question,
          createdAt: Date.now(), status: 'open',
          title: qs[0]?.header || 'Question', detail: qs[0]?.question,
          payload: { type: 'question', questions: qs },
        });
      }

      if (enabled('done')) {
        const doneAt = doneAtRef.current.get(sid);
        if (doneAt && !snap.pendingApproval && !(snap.pendingQuestions?.length)) {
          const sig = `${sid}:done:${doneAt}`;
          out.push({
            ...base, id: sig, signature: sig, kind: 'done', priority: KIND_PRIORITY.done,
            createdAt: doneAt, status: 'open',
            title: 'Finished', detail: lastAssistant(snap) || 'Agent is idle and ready for review',
            payload: { type: 'summary', summary: lastAssistant(snap) },
          });
        }
      }
    }
    const open = out.filter((it) => !dismissed.has(it.signature) && (snoozedUntil.get(it.signature) ?? 0) <= now);
    return sortItems(open);
  // doneTick forces recompute when doneAtRef mutates.
  }, [snapshotBySession, agents, dismissed, snoozedUntil, now, doneTick]); // eslint-disable-line react-hooks/exhaustive-deps

  const counts = useMemo(() => {
    const byKind = { approval: 0, question: 0, stuck: 0, error: 0, done: 0, bigdiff: 0 } as Record<AttentionKind, number>;
    let needsYou = 0;
    for (const it of items) {
      byKind[it.kind]++;
      if (it.kind === 'approval' || it.kind === 'question' || it.kind === 'stuck' || it.kind === 'error') needsYou++;
    }
    return { total: items.length, needsYou, byKind };
  }, [items]);

  const dismiss = useCallback((signature: string) => {
    setDismissed((prev) => { const n = new Set(prev); n.add(signature); return n; });
  }, []);
  const snooze = useCallback((signature: string, minutes: number) => {
    setSnoozedUntil((prev) => { const n = new Map(prev); n.set(signature, Date.now() + minutes * 60_000); return n; });
  }, []);

  return { items, counts, dismiss, snooze };
}
