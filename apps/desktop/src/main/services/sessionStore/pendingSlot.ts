import type { ClaudeSessionState, PendingApproval, PendingQuestion } from '../claudeSessionStore';

// ── The pending slot ──────────────────────────────────────────────────────────
//
// INVARIANT: exactly one feed owns a session's pending slot (`pendingApproval`
// + `pendingQuestions`). Other feeds may ENRICH the session, but must never
// park or resolve that slot. Every worker-freeze this project has shipped came
// from breaking it: a card nulled by the feed that did not own it, while the
// owning feed still held the request, leaves a session visibly blocked with
// nothing to answer and no way out but killing it.
//
// This module is the desktop's single statement of the rule — who owns the
// slot, and the only vocabulary in which anyone may write it. `hookEventRouter`
// (the hook feed) and `claudeSessionStore` (the daemon feed, the federation
// feed, and the answer acknowledgement) all go through it, so the two cannot
// drift into disagreeing about who owns a card.
//
// It mirrors claudemon's `PendingWrite` (3064726c), with one word the Rust side
// does not need — see {@link acknowledgeAnswer}.

/**
 * Which feed owns a session's pending slot.
 *
 *   • `hooks` — claude on the PTY transport. `PermissionRequest` /
 *     `AskUserQuestion` hooks are the only source of its cards, and the
 *     matching PostToolUse/Stop are the only thing that clears them.
 *   • `daemon` — any non-claude provider, or claude on the stream transport.
 *     Cards come from claudemon's single `pending` slot on the `/events` SSE
 *     stream (claudemonEventBridge → applyManagedMode → applyManagedPending);
 *     the daemon resolves them on the turn-closing `result` frame.
 *   • `federation` — a row that arrived FROM a peer hub. Its slot is a mirror
 *     of the peer's, refreshed by `upsertRemoteSession` /
 *     `upsertSparseRemoteSession`; nothing local holds the request, so no local
 *     feed may park or resolve it.
 */
export type PendingFeed = 'hooks' | 'daemon' | 'federation';

/** Who owns this session's slot. `hub` first: a remote row is a mirror no
 *  matter what provider/transport the peer reported for it, and the local
 *  daemon/hook feeds must keep their hands off it. (The reverse direction —
 *  a peer overwriting a local row — is already refused in
 *  `upsertRemoteSession`; this is the missing half of that guard.) */
export function pendingSlotOwner(
  session: Pick<ClaudeSessionState, 'provider' | 'transport' | 'hub'>,
): PendingFeed {
  if (session.hub) return 'federation';
  const isClaude = (session.provider ?? 'claude') === 'claude';
  return isClaude && session.transport !== 'stream' ? 'hooks' : 'daemon';
}

/** A session as any feed may mutate it: everything writable EXCEPT the pending
 *  slot, which is readonly so the compiler routes every write through
 *  {@link PendingSlot}. `readonly` is not checked in assignability, so a value
 *  of this type still passes anywhere a `ClaudeSessionState` is wanted (the
 *  notifier, the watcher, `applyHookEvent`, `publishSnapshot`) — it constrains
 *  the code holding it, not the code it hands the session to. */
export type PendingFencedSession = Omit<
  ClaudeSessionState,
  'pendingApproval' | 'pendingQuestions'
> & {
  readonly pendingApproval: ClaudeSessionState['pendingApproval'];
  readonly pendingQuestions: ClaudeSessionState['pendingQuestions'];
};

/** A session row under construction, before it has a pending slot. Naming the
 *  two fields in a literal of this type is an excess property (TS2353) — the
 *  construction-side half of the fence, because `readonly` only forbids
 *  assignment AFTER initialization and an object literal may still initialize
 *  it. Rows are born through {@link bornWithPending} instead. */
export type SessionWithoutPending = Omit<
  ClaudeSessionState,
  'pendingApproval' | 'pendingQuestions'
>;

function sameApproval(a: PendingApproval | null, b: PendingApproval): boolean {
  return (
    !!a && a.toolName === b.toolName && JSON.stringify(a.toolInput) === JSON.stringify(b.toolInput)
  );
}

/**
 * A feed's handle on one session's pending slot, mirroring claudemon's
 * `PendingWrite` intent: a write either PARKS a decision or RESOLVES one, and
 * "keep it as it is" is expressed by not calling at all. Every park and resolve
 * is suppressed unless the constructing feed owns the slot.
 *
 * Each method returns what the slot HOLDS after the call — like
 * `set_managed_mode`, so a caller mirrors the real state rather than assuming
 * its own write landed.
 */
export class PendingSlot {
  private readonly owned: boolean;

  constructor(
    private readonly session: ClaudeSessionState,
    feed: PendingFeed,
  ) {
    this.owned = pendingSlotOwner(session) === feed;
  }

  /** Whether this feed's writes land at all. Read it to log or branch; the
   *  writes below already refuse on their own. */
  get ownsSlot(): boolean {
    return this.owned;
  }

  /**
   * PARK an approval this feed is holding open.
   *
   * An unchanged card keeps the object it already had, timestamp included:
   * every producer re-sends the same parked request on unrelated state changes
   * (the daemon rebroadcasts Approval-mode frames; a brain row carries no
   * timestamp at all), and re-stamping it resurrects a card the user already
   * dismissed — the needs-you dock hides on dismissal timestamps. That is
   * claudemon's `Keep`, folded in here so no caller has to remember it.
   */
  parkApproval(next: PendingApproval): PendingApproval | null {
    if (this.owned && !sameApproval(this.session.pendingApproval, next)) {
      this.session.pendingApproval = next;
    }
    return this.session.pendingApproval;
  }

  /** PARK a question set. Unchanged sets keep the array they had, for the same
   *  reason `parkApproval` keeps the card: the renderer keys off identity. */
  parkQuestions(next: PendingQuestion[]): PendingQuestion[] | null {
    const prev = this.session.pendingQuestions;
    if (this.owned && JSON.stringify(prev) !== JSON.stringify(next)) {
      this.session.pendingQuestions = next;
    }
    return this.session.pendingQuestions;
  }

  /** RESOLVE the approval — this feed says the request it held is gone. */
  resolveApproval(): PendingApproval | null {
    if (this.owned) this.session.pendingApproval = null;
    return this.session.pendingApproval;
  }

  /** RESOLVE the question set. */
  resolveQuestions(): PendingQuestion[] | null {
    if (this.owned) this.session.pendingQuestions = null;
    return this.session.pendingQuestions;
  }

  /** RESOLVE both halves — a turn boundary, or a producer saying "nothing is
   *  pending". Still ownership-gated: one feed's turn end says nothing about
   *  what another feed is holding open (the hook `Stop` that stranded a
   *  background subagent's `can_use_tool` was exactly this). */
  resolveAll(): void {
    this.resolveApproval();
    this.resolveQuestions();
  }
}

/**
 * The word the hook feed does not have, and the reason the owning feed needed
 * its own vocabulary rather than a second copy of `HookPendingSlot`.
 *
 * The user answered, and the owner of the request ACCEPTED the answer
 * (`POST /answer` returned, or the peer's `claude.answer` did). Clearing the
 * picker here is therefore not one feed guessing about another's state — it is
 * the resolution of that exact request, arriving before the confirmation that
 * will independently clear it (the PostToolUse hook on PTY, the next managed
 * frame on stream/managed, the peer's next snapshot when remote).
 *
 * So it takes no {@link PendingFeed} and is deliberately UNGATED — and it is a
 * free function rather than a {@link PendingSlot} method precisely so nobody
 * reads it as the pattern for reaching past the gate. It is narrow, which is
 * what makes being ungated safe:
 *   • questions only. An approval is answered through `claude.approve`, whose
 *     decision the owner may still reject as unknown; clearing that card
 *     optimistically would hide a request that is still open.
 *   • a no-op when nothing is parked, so it can never be the thing that empties
 *     a slot some other feed just filled.
 */
export function acknowledgeAnswer(session: PendingFencedSession): PendingQuestion[] | null {
  const live = session as ClaudeSessionState;
  if (live.pendingQuestions) live.pendingQuestions = null;
  return live.pendingQuestions;
}

/**
 * Give a freshly-built row its pending slot, stating the intent.
 *
 * Rows rebuilt wholesale from a wire snapshot (the federation feeds) never
 * *assign* to the slot, so `readonly` alone cannot fence them — they would just
 * name the fields in the literal. Typing that literal {@link
 * SessionWithoutPending} makes naming them an error, and this is the one door
 * left: seed from what the row previously held (so `parkApproval`'s Keep rule
 * can still see the old card and preserve its timestamp), then let the feed
 * declare what the wire actually says.
 */
export function bornWithPending(
  draft: SessionWithoutPending,
  feed: PendingFeed,
  previous: Pick<ClaudeSessionState, 'pendingApproval' | 'pendingQuestions'> | undefined,
  fill: (slot: PendingSlot) => void,
): ClaudeSessionState {
  const session = draft as ClaudeSessionState;
  session.pendingApproval = previous?.pendingApproval ?? null;
  session.pendingQuestions = previous?.pendingQuestions ?? null;
  fill(new PendingSlot(session, feed));
  return session;
}

/** A row at birth: the slot starts empty and no feed owns anything yet — the
 *  first hook or managed frame parks through its own slot. Spelled out so the
 *  two fields never have to appear in a session literal at all. */
export function bornWithEmptyPending(draft: SessionWithoutPending): PendingFencedSession {
  return bornWithPending(draft, 'hooks', undefined, () => {});
}
