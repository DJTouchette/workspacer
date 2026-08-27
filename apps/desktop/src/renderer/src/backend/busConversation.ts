/**
 * Conversation folding for the bus backend — the transcript a headless session
 * has and its snapshot doesn't.
 *
 * Two providers publish `agent.snapshot` rows (see webBackend's snapshot fold):
 * the desktop app publishes rich `ClaudeSessionSnapshot`s that carry a
 * conversation window, and the headless brain publishes claudemon's own session
 * row, marked `sparse: true`. A sparse row deliberately carries **no
 * conversation at all** — brain's parity record says so in as many words: "the
 * turn-by-turn transcript lives in claudemon's /conversation endpoint, not the
 * session row; folding it into every snapshot/publish would ship whole
 * transcripts per state tick". Clients are expected to fetch it themselves;
 * `/m` polls `sessions.conversation` for exactly this reason.
 *
 * The web renderer never did, so against a hub whose sessions come from a
 * headless node (`workspacer serve`, a Fly worker node) `/app` rendered an
 * empty transcript: no assistant text, no tool cards — and, because
 * ClaudePane retires its optimistic "Sending…" bubble by watching
 * `session.conversation` grow a user turn, that bubble never cleared. The send
 * itself was fine (`agents.sendMessage` acks in ~2ms and the turn runs); the
 * client simply had no way to see it land.
 *
 * This module is the client-side half: fetch `sessions.conversation`, fold its
 * items into renderer `ConversationTurn`s, and hand them to the snapshot path.
 *
 * Ported from `main/services/sessionStore/conversationApplier.ts` — same wire
 * shape, same dedup rules, same "each tool call is its own turn" timeline —
 * minus what only a full session store can do (usage accounting, hook-tracked
 * tool reaping, file-change recording; a sparse row already carries claudemon's
 * usage/tool counters). Keep the two in step: they render the same transcript.
 */

// The MAIN-shared shapes, not the renderer's near-twin: webBackend speaks
// `main/shared/ipcTypes` across the whole seam (a turn's `timestamp` is
// optional there, which is exactly what a daemon item without one folds to).
import type {
  ClaudeSessionSnapshot,
  ConversationTurn,
  SessionPlan,
  PlanStep,
  ToolCall,
} from '../../../main/shared/ipcTypes';

/** Wire shape of one item from claudemon's `ConversationItem` enum
 *  (services/claudemon/src/session/conversation.rs). */
export interface ConversationItemWire {
  kind?: string;
  /** Some payloads tag the discriminant as `type` rather than `kind`. */
  type?: string;
  text?: string;
  id?: string;
  name?: string;
  input?: any;
  args?: string;
  output?: string;
  tool_use_id?: string;
  content?: string;
  is_error?: boolean;
  steps?: Array<{ content?: string; status?: string; activeForm?: string; active_form?: string }>;
  updatedAt?: number | string;
  updated_at?: number | string;
  timestamp?: string;
}

/** Wire shape of `sessions.conversation` (claudemon's `/conversation`). */
export interface ConversationSnapWire {
  seq?: number;
  first_seq?: number;
  items?: ConversationItemWire[];
}

/**
 * Turn cap, matching `MAX_CONVERSATION_TURNS` in sessionStore/bounds.ts. A
 * long-lived agent grows this array forever (every tool call is a turn), and
 * the renderer holds it per session for the tab's whole life.
 */
const MAX_TURNS = 2_000;

/** Per-session fold state: the turns themselves plus what the incremental
 *  fetch needs to ask the right question next time. */
export interface ConversationState {
  turns: ConversationTurn[];
  /** Tool-call ids already in `turns` — a re-delivered tool_use is dropped. */
  toolIds: Set<string>;
  plan?: SessionPlan;
  /** Turns dropped off the front by the cap (absolute index = offset + i). */
  offset: number;
  /** User sends among the dropped turns — ClaudePane's optimistic bubbles
   *  retire against an ABSOLUTE user-send tally, so this must be banked. */
  userOffset: number;
  /** Highest `seq` the daemon reported (its delta counter, not an index). */
  lastSeq: number;
  /** Seq-space index of the newest item we hold, or -1 when we hold none. */
  lastItemSeq: number;
  inflight: Promise<ConversationSnapWire | null> | null;
  /** A poke that arrived while a fetch was in flight — run one more after. */
  queued: boolean;
}

/** A fresh, empty fold state (exported for tests). */
export function newConversationState(): ConversationState {
  return {
    turns: [],
    toolIds: new Set(),
    offset: 0,
    userOffset: 0,
    lastSeq: -1,
    lastItemSeq: -1,
    inflight: null,
    queued: false,
  };
}

function tsOf(item: ConversationItemWire): number {
  if (item.timestamp) {
    const ms = Date.parse(item.timestamp);
    if (!Number.isNaN(ms)) return ms;
  }
  return Date.now();
}

/** conversationApplier's `isDuplicateMessage`, verbatim in intent: claude's
 *  JSONL repeats rows around compaction, and our incremental fetch deliberately
 *  re-requests the newest item so its in-place growth reaches us. */
function isDuplicateMessage(
  turns: ConversationTurn[],
  role: string,
  content: string,
  timestamp?: number,
): boolean {
  if (!content) return false;
  return turns
    .slice(-5)
    .some(
      (t) =>
        t.role === role &&
        !!t.content &&
        t.content === content &&
        (timestamp === undefined || t.timestamp === timestamp),
    );
}

function normalizePlanSteps(raw: ConversationItemWire['steps']): PlanStep[] {
  if (!Array.isArray(raw)) return [];
  const steps: PlanStep[] = [];
  for (const s of raw) {
    const content = typeof s?.content === 'string' ? s.content : '';
    if (!content) continue;
    const status = s.status === 'in_progress' || s.status === 'completed' ? s.status : 'pending';
    const activeForm = s.activeForm ?? s.active_form;
    steps.push(activeForm ? { content, status, activeForm } : { content, status });
  }
  return steps;
}

function countUserTurns(turns: ConversationTurn[]): number {
  // The same definition main/shared/conversationCount.ts uses: a synthetic
  // name-less command card (an orphaned command_output) is not a user send.
  return turns.filter((t) => t.role === 'user' && !(t.command && t.command.name === '')).length;
}

/**
 * Fold a batch of wire items into a session's turns, in place.
 *
 * Exported for tests: the failure mode is a transcript that is wrong (or
 * missing) on the session the user is actively watching.
 */
export function applyConversationItems(
  state: ConversationState,
  items: ConversationItemWire[],
): void {
  for (const item of items) {
    const kind = item.kind ?? item.type;
    switch (kind) {
      case 'user_message': {
        const text = item.text ?? '';
        if (!text) break;
        const ts = item.timestamp ? tsOf(item) : undefined;
        if (!isDuplicateMessage(state.turns, 'user', text, ts)) {
          // `timestamp` stays undefined when the daemon sent none — the same
          // shape the desktop store pushes, and ClaudePane's stream-echo
          // convergence keys on that absence.
          state.turns.push({ role: 'user', content: text, timestamp: ts });
        }
        break;
      }

      case 'assistant_text': {
        const text = item.text ?? '';
        if (!text) break;
        // claudemon coalesces a streamed reply into ONE item that grows in
        // place, and our poll re-requests the newest item precisely so that
        // growth arrives. Extend the bubble instead of pushing a second one.
        const last = state.turns[state.turns.length - 1];
        if (last && last.role === 'assistant' && !last.toolCalls?.length) {
          if (!last.content || text.startsWith(last.content)) {
            last.content = text;
            break;
          }
          if (last.content === text) break;
        }
        if (!isDuplicateMessage(state.turns, 'assistant', text)) {
          state.turns.push({ role: 'assistant', content: text, timestamp: tsOf(item) });
        }
        break;
      }

      case 'slash_command': {
        const name = item.name ?? '';
        if (!name) break;
        const args = item.args ?? '';
        const recent = state.turns.slice(-5);
        if (
          recent.some(
            (t) => t.command && t.command.name === name && (t.command.args ?? '') === args,
          )
        )
          break;
        state.turns.push({
          role: 'user',
          content: `/${name}${args ? ` ${args}` : ''}`,
          timestamp: tsOf(item),
          command: args ? { name, args } : { name },
        });
        break;
      }

      case 'command_output': {
        const output = item.output ?? '';
        if (!output) break;
        let attached = false;
        const floor = Math.max(0, state.turns.length - 10);
        for (let i = state.turns.length - 1; i >= floor; i--) {
          const turn = state.turns[i];
          if (!turn.command) continue;
          if (turn.command.output == null) {
            turn.command.output = output;
            if (item.is_error) turn.command.outputIsError = true;
            attached = true;
            break;
          }
          if (turn.command.output === output) {
            attached = true; // replay of the same output
            break;
          }
        }
        if (!attached) {
          state.turns.push({
            role: 'user',
            content: output,
            timestamp: tsOf(item),
            command: item.is_error
              ? { name: '', output, outputIsError: true }
              : { name: '', output },
          });
        }
        break;
      }

      case 'tool_use': {
        if (item.id && state.toolIds.has(item.id)) break;
        const ts = tsOf(item);
        const tc: ToolCall = {
          id: item.id || `tc-${ts}-${state.turns.length}`,
          name: item.name ?? 'unknown',
          input: item.input ?? {},
          status: 'complete',
          startedAt: ts,
          completedAt: ts,
        };
        state.toolIds.add(tc.id);
        // Each tool call is its own turn — interlaced with text in timeline
        // order, exactly as the desktop store builds it.
        state.turns.push({ role: 'assistant', content: '', timestamp: ts, toolCalls: [tc] });
        // Fallback for a daemon that emits no dedicated `plan` item: Claude's
        // TodoWrite call carries the whole checklist in its input.
        if (item.name === 'TodoWrite' && Array.isArray(item.input?.todos)) {
          const steps = normalizePlanSteps(item.input.todos);
          if (steps.length > 0) state.plan = { steps, updatedAt: ts };
        }
        break;
      }

      case 'tool_result': {
        if (!item.tool_use_id) break;
        for (let i = state.turns.length - 1; i >= 0; i--) {
          const tcs = state.turns[i].toolCalls;
          if (!tcs) continue;
          const tc = tcs.find((t) => t.id === item.tool_use_id);
          if (!tc) continue;
          tc.response = item.content ?? '';
          if (item.is_error) tc.status = 'failed';
          if (item.timestamp) {
            const doneMs = Date.parse(item.timestamp);
            if (!Number.isNaN(doneMs) && doneMs >= tc.startedAt) tc.completedAt = doneMs;
          }
          break;
        }
        break;
      }

      case 'plan': {
        const steps = normalizePlanSteps(item.steps);
        const stamp = item.updatedAt ?? item.updated_at;
        const updatedAt =
          typeof stamp === 'number'
            ? stamp
            : typeof stamp === 'string' && !Number.isNaN(Date.parse(stamp))
              ? Date.parse(stamp)
              : tsOf(item);
        state.plan = { steps, updatedAt };
        break;
      }

      // `usage` is deliberately not folded: a sparse row already carries
      // claudemon's own usage/context counters, which is where the renderer
      // reads them from.
    }
  }

  const excess = state.turns.length - MAX_TURNS;
  if (excess > 0) {
    const dropped = state.turns.splice(0, excess);
    state.offset += excess;
    state.userOffset += countUserTurns(dropped);
  }
}

/**
 * Cheap "did this fold change anything the UI would show" fingerprint. The
 * three things a fetch can move are the turn count, the newest bubble's text
 * (a streaming reply grows in place) and the newest tool call's result.
 */
function foldSignature(st: ConversationState): string {
  const last = st.turns[st.turns.length - 1];
  const tc = last?.toolCalls?.[0];
  return [
    st.turns.length,
    st.offset,
    last?.content?.length ?? 0,
    last?.command?.output?.length ?? 0,
    tc ? `${tc.status}:${String(tc.response ?? '').length}` : '',
    st.plan?.updatedAt ?? 0,
  ].join('|');
}

export interface BusConversations {
  /**
   * Overlay the folded transcript onto a snapshot that has none (a sparse
   * brain row). A snapshot that carries its own `conversation` — every rich
   * desktop row — is returned untouched.
   */
  merge(snap: ClaudeSessionSnapshot): ClaudeSessionSnapshot;
  /** Fetch this session's conversation delta and fold it in. Coalesced: one
   *  fetch in flight per session, later pokes collapse into a single trailing
   *  one (the same guard shape as lib/federation's remote sync). */
  poke(sessionId: string): Promise<ConversationSnapWire | null>;
  /** Drop a session's transcript (it ended — nothing more is coming). */
  forget(sessionId: string): void;
  /** True once we hold state for this session (test/introspection helper). */
  has(sessionId: string): boolean;
}

/**
 * @param call    issues one bus RPC (already hub-qualified by the caller)
 * @param onFold  called after a fetch actually changed a session's transcript,
 *                so the backend can re-emit that session's snapshot
 */
export function createBusConversations(
  call: (sessionId: string, params: Record<string, unknown>) => Promise<unknown>,
  onFold: (sessionId: string) => void,
): BusConversations {
  const states = new Map<string, ConversationState>();

  const merge = (snap: ClaudeSessionSnapshot): ClaudeSessionSnapshot => {
    if (Array.isArray(snap?.conversation)) return snap;
    const st = states.get(snap?.sessionId);
    if (!st) return snap;
    return {
      ...snap,
      // A COPY, deliberately, and the cost (a pointer per turn, capped at
      // MAX_TURNS) is the price of the renderer's snapshot contract. `st.turns`
      // is a fold buffer mutated in place, so handing it out directly gives
      // every snapshot the SAME array identity forever — and the renderer
      // memoizes on exactly that identity. ClaudePane's `lastUserTs` (the
      // "Working for 1m 04s" anchor) is `useMemo(..., [conversation])`: against
      // a headless fleet it never recomputed, so every turn was anchored to the
      // PREVIOUS turn's user message and the label counted the idle gaps
      // between turns as work. Over Electron IPC the desktop gets a fresh array
      // per push for free (structured clone); this is that guarantee, restored.
      conversation: st.turns.slice(),
      conversationOffset: st.offset,
      conversationUserOffset: st.userOffset,
      ...(st.plan && !snap.plan ? { plan: st.plan } : {}),
    } as ClaudeSessionSnapshot;
  };

  const fetchOnce = async (sessionId: string): Promise<ConversationSnapWire | null> => {
    const st = states.get(sessionId)!;
    // Re-request the NEWEST item we already hold (lastItemSeq - 1), not
    // everything after it: claudemon coalesces a streaming reply into one item
    // that grows in place while `seq` races ahead of it, so asking for
    // "strictly newer than what I have" would never see the reply grow.
    const since = st.lastItemSeq >= 0 ? Math.max(0, st.lastItemSeq - 1) : undefined;
    let res = (await call(sessionId, {
      sessionId,
      ...(since !== undefined && { sinceSeq: since }),
    })) as ConversationSnapWire | null;
    if (!res || !Array.isArray(res.items)) return null;
    let items: ConversationItemWire[] = res.items;

    let usedSince = since;
    // A seq that went BACKWARDS means the daemon rebuilt this session's log
    // (a managed provider restarted the thread). Refetch whole and replace —
    // folding a fresh thread onto the old one interleaves two conversations.
    if (typeof res.seq === 'number' && st.lastSeq >= 0 && res.seq < st.lastSeq) {
      const full = (await call(sessionId, { sessionId })) as ConversationSnapWire | null;
      if (!full || !Array.isArray(full.items)) return null;
      res = full;
      items = full.items;
      usedSince = undefined;
      st.turns = [];
      st.toolIds.clear();
      st.offset = 0;
      st.userOffset = 0;
      st.plan = undefined;
    }

    const before = foldSignature(st);
    applyConversationItems(st, items);
    const after = foldSignature(st);

    if (typeof res.seq === 'number') st.lastSeq = res.seq;
    if (items.length > 0) {
      // Seq-space index of the newest item: `since + n` for a delta,
      // `first_seq + n - 1` for a whole-window answer (see items_skip in
      // claudemon's api.rs — the two forms agree).
      st.lastItemSeq =
        usedSince !== undefined
          ? usedSince + items.length
          : (res.first_seq ?? 1) + items.length - 1;
    }
    if (before !== after) onFold(sessionId);
    return res;
  };

  const poke = (sessionId: string): Promise<ConversationSnapWire | null> => {
    if (!sessionId) return Promise.resolve(null);
    let st = states.get(sessionId);
    if (!st) {
      st = newConversationState();
      states.set(sessionId, st);
    }
    if (st.inflight) {
      st.queued = true;
      return st.inflight;
    }
    const run = fetchOnce(sessionId)
      .catch(() => null)
      .then((res) => {
        const cur = states.get(sessionId);
        if (cur) {
          cur.inflight = null;
          if (cur.queued) {
            cur.queued = false;
            void poke(sessionId);
          }
        }
        return res;
      });
    st.inflight = run;
    return run;
  };

  return {
    merge,
    poke,
    forget: (sessionId) => {
      states.delete(sessionId);
    },
    has: (sessionId) => states.has(sessionId),
  };
}
