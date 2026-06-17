import { useEffect, useMemo, useRef, useState, useCallback } from 'react';
import type { AgentWorkspace } from '../types/pane';
import type { AttentionItem, AttentionKind } from '../types/attention';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import { KIND_PRIORITY, sortItems } from '../lib/attentionRouter';
import { usePageVisible } from './usePageVisible';

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

/** Newlines in a string field, used to estimate a diff's line count. */
function lineCount(s: unknown): number {
  if (typeof s !== 'string' || !s) return 0;
  return s.split('\n').length;
}

/** Rough added+removed line estimate across an agent's file changes. */
function diffSize(snap: ClaudeSessionSnapshot): { lines: number; files: number } {
  const changes = snap.fileChanges ?? [];
  const files = new Set<string>();
  let lines = 0;
  for (const ch of changes) {
    files.add(ch.path);
    const inp = ch.input ?? {};
    // Edit: new_string + old_string; Write: content. Multi-edit: edits[].
    lines += lineCount(inp.new_string) + lineCount(inp.old_string) + lineCount(inp.content);
    if (Array.isArray(inp.edits)) {
      for (const e of inp.edits) lines += lineCount(e?.new_string) + lineCount(e?.old_string);
    }
  }
  return { lines, files: files.size };
}

/** Idle/threshold knobs for the heuristic kinds — kept conservative to avoid noise. */
const BIGDIFF_LINES = 80;          // added+removed lines before a review nudge
const STUCK_MS = 5 * 60_000;       // an unanswered question idle this long is "stuck"
const ERROR_RECENT_MS = 5 * 60_000; // only surface a trailing tool error this fresh

export interface AttentionFeed {
  items: AttentionItem[];
  counts: { total: number; needsYou: number; byKind: Record<AttentionKind, number> };
  /** agentId → that agent's single most-urgent open item (priority order). Shared
   *  by the Fleet Deck (card buoyancy) and the SideBar (per-row dot + glyph). */
  topByAgent: Map<string, AttentionItem>;
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
  const pageVisible = usePageVisible();

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

  // Prune per-session memory for sessions that no longer exist (terminated
  // agents / ended sessions). Without this the done-detection refs and the
  // dismissed/snoozed sets grow unbounded across a long-running app. Keyed on
  // the live session set so it runs whenever sessions appear or disappear.
  const liveSessionKey = Object.keys(snapshotBySession).sort().join(',');
  useEffect(() => {
    const live = new Set(Object.keys(snapshotBySession));
    for (const sid of prevStateRef.current.keys()) if (!live.has(sid)) prevStateRef.current.delete(sid);
    for (const sid of doneAtRef.current.keys()) if (!live.has(sid)) doneAtRef.current.delete(sid);
    // dismissed/snoozed are keyed by item signature, which is prefixed with the
    // sessionId (`${sid}:kind:…`), so a dead session's entries start with `sid:`.
    setDismissed((prev) => {
      let mutated = false;
      const n = new Set<string>();
      for (const sig of prev) {
        if (live.has(sig.split(':')[0])) n.add(sig); else mutated = true;
      }
      return mutated ? n : prev;
    });
    setSnoozedUntil((prev) => {
      let mutated = false;
      const n = new Map<string, number>();
      for (const [sig, t] of prev) {
        if (live.has(sig.split(':')[0])) n.set(sig, t); else mutated = true;
      }
      return mutated ? n : prev;
    });
  }, [liveSessionKey]); // eslint-disable-line react-hooks/exhaustive-deps

  // Snooze-expiry ticker (also re-surfaces snoozed items when their time is up).
  // Only armed when something is actually snoozed — with nothing snoozed there's
  // nothing whose expiry could change the feed, so the timer would burn CPU
  // recomputing the same feed every 5s forever. Also paused while the window is
  // hidden (the app should idle toward ~0% CPU when switched away).
  useEffect(() => {
    if (snoozedUntil.size === 0 || !pageVisible) return;
    const i = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(i);
  }, [snoozedUntil.size, pageVisible]);

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

      const idle = snap.ambientState === 'idle' || snap.ambientState === 'waiting_input';
      const blocked = !!snap.pendingApproval || !!(snap.pendingQuestions?.length);

      if (enabled('done')) {
        const doneAt = doneAtRef.current.get(sid);
        if (doneAt && !blocked) {
          const sig = `${sid}:done:${doneAt}`;
          out.push({
            ...base, id: sig, signature: sig, kind: 'done', priority: KIND_PRIORITY.done,
            createdAt: doneAt, status: 'open',
            title: 'Finished', detail: lastAssistant(snap) || 'Agent is idle and ready for review',
            payload: { type: 'summary', summary: lastAssistant(snap) },
          });
        }
      }

      // bigdiff — an idle agent left a large unreviewed change behind.
      if (enabled('bigdiff') && idle && !blocked) {
        const { lines, files } = diffSize(snap);
        if (lines > BIGDIFF_LINES) {
          const sig = `${sid}:bigdiff:${files}:${Math.round(lines / 20)}`;
          out.push({
            ...base, id: sig, signature: sig, kind: 'bigdiff', priority: KIND_PRIORITY.bigdiff,
            createdAt: doneAtRef.current.get(sid) ?? snap.lastActivity ?? Date.now(), status: 'open',
            title: 'Large change to review',
            detail: `${files} file${files === 1 ? '' : 's'}, ±${lines} lines`,
            payload: { type: 'summary', summary: `${files} files, ±${lines} lines` },
          });
        }
      }

      // stuck — a question that's been sitting unanswered for a while.
      if (enabled('stuck') && snap.pendingQuestions?.length) {
        const since = snap.lastActivity ?? 0;
        if (since && now - since > STUCK_MS) {
          const sig = `${sid}:stuck:${hash(snap.pendingQuestions.map((q) => q.question).join('|'))}`;
          out.push({
            ...base, id: sig, signature: sig, kind: 'stuck', priority: KIND_PRIORITY.stuck,
            createdAt: since, status: 'open',
            title: 'Waiting on you',
            detail: snap.pendingQuestions[0]?.question || 'An unanswered question is holding this agent.',
            payload: { type: 'summary', summary: 'Agent has been waiting for a while' },
          });
        }
      }

      // error — the agent's most recent tool call failed.
      if (enabled('error') && idle && !blocked) {
        const calls = snap.completedToolCalls ?? [];
        const last = calls[calls.length - 1];
        if (last && last.status === 'failed') {
          const at = last.completedAt ?? last.startedAt ?? snap.lastActivity ?? Date.now();
          if (Date.now() - at < ERROR_RECENT_MS) {
            const sig = `${sid}:error:${last.id}`;
            out.push({
              ...base, id: sig, signature: sig, kind: 'error', priority: KIND_PRIORITY.error,
              createdAt: at, status: 'open',
              title: `${last.name} failed`,
              detail: typeof last.response === 'string' ? last.response.split('\n')[0].slice(0, 120) : 'Last tool call errored.',
              payload: { type: 'summary', summary: `${last.name} failed` },
            });
          }
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

  // items is already sorted most-urgent-first, so the first hit per agent wins.
  const topByAgent = useMemo(() => {
    const m = new Map<string, AttentionItem>();
    for (const it of items) if (!m.has(it.agentId)) m.set(it.agentId, it);
    return m;
  }, [items]);

  const dismiss = useCallback((signature: string) => {
    setDismissed((prev) => { const n = new Set(prev); n.add(signature); return n; });
  }, []);
  const snooze = useCallback((signature: string, minutes: number) => {
    setSnoozedUntil((prev) => { const n = new Map(prev); n.set(signature, Date.now() + minutes * 60_000); return n; });
  }, []);

  return { items, counts, topByAgent, dismiss, snooze };
}
