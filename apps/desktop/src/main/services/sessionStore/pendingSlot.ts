import type {
  ClaudeSessionState,
  PendingApproval,
  PendingQuestion,
  PendingQuestionOption,
} from '../claudeSessionStore';

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

/** The parked payloads, frozen at the type level. Deep enough that every way
 *  of writing one is an error and not just the outermost assignment:
 *  `q.question = …` and `q.options.push(…)` are as dead as `q.options = []`. */
type ReadOnlyPendingApproval = Readonly<PendingApproval>;
type ReadOnlyPendingQuestion = Readonly<Omit<PendingQuestion, 'options'>> & {
  readonly options: readonly Readonly<PendingQuestionOption>[];
};

/**
 * A session as a COLLABORATOR OUTSIDE the store sees it: free to read the
 * pending slot and free to enrich everything else, unable to touch the slot at
 * all.
 *
 * This is the fence past the file boundary. {@link PendingFencedSession} fences
 * the store's own class body, but `this.sessions.values()` hands live rows to
 * the notifier, the supervisor nudger, the usage accumulator, the conversation
 * applier, the budget watcher and the analytics writer — six modules holding a
 * mutable reference to a fenced row, which today only READ the slot. This type
 * is what makes that a compiler fact rather than a code-review promise.
 *
 * Deliberately STRONGER than `PendingFencedSession`, and the difference matters:
 * a bare `readonly` field forbids `session.pendingQuestions = …` (TS2540) but
 * says nothing about `session.pendingQuestions.push(…)`, which reaches the same
 * array the store is holding. Here the array is `readonly T[]`, so `push`,
 * `splice` and `[0] = …` do not exist (TS2339 / TS2542).
 *
 * The price is that a `PendingReadOnlySession` is NOT assignable back to
 * `ClaudeSessionState` — a `readonly T[]` is not a `T[]`. That is the point: a
 * fenced collaborator cannot launder the row by handing it to something that
 * takes the mutable type. Store rows and plain `ClaudeSessionState` values flow
 * INTO it freely (mutable is assignable to readonly), so call sites are
 * unchanged.
 */
export type PendingReadOnlySession = Omit<
  ClaudeSessionState,
  'pendingApproval' | 'pendingQuestions'
> & {
  readonly pendingApproval: ReadOnlyPendingApproval | null;
  readonly pendingQuestions: readonly ReadOnlyPendingQuestion[] | null;
};

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
  /** The one place the fence is widened back to a mutable row. Every fenced
   *  type above is assignable INTO this constructor, so the gated door is open
   *  to the store, to the hook feed and to any fenced collaborator that ever
   *  earns a legitimate write — and the cast lives here rather than at each of
   *  their call sites, where it would read as permission. */
  private readonly session: ClaudeSessionState;

  constructor(session: PendingReadOnlySession, feed: PendingFeed) {
    this.session = session as ClaudeSessionState;
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
export function acknowledgeAnswer(session: PendingReadOnlySession): PendingQuestion[] | null {
  const live = session as unknown as ClaudeSessionState;
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

/**
 * A DETACHED copy of a session's pending slot, for handing outside the store.
 *
 * `getSnapshot`/`getAllSnapshots` shallow-clone the row, which left
 * `snap.pendingQuestions` pointing at the store's own array and
 * `snap.pendingApproval` at the store's own card. A caller never had to name
 * either field to corrupt them: `snap.pendingQuestions.push(…)`,
 * `.length = 0`, or `card.toolInput.command = …` reached past every fence
 * above — no assignment, so no `readonly` could have caught it, and no
 * `PendingSlot`, so no ownership check ran. Emptying the array is the freeze
 * shape; rewriting the card is worse, because the Approve button still works
 * and just approves something other than what is shown.
 *
 * So the slot is COPIED out rather than typed out. A type would have had to
 * reach the IPC boundary's `ClaudeSessionSnapshot` (main/shared/ipcTypes) and
 * the renderer's copy of it to mean anything, and it would still only bind
 * code that opts into the type — whereas a caller mutating a detached copy
 * simply cannot reach the store, cast or no cast. Cheap where it matters: an
 * unblocked session has nothing parked and both branches return `null`.
 *
 * Used at every point a snapshot leaves the store: `getSnapshot`,
 * `getAllSnapshots`, and the `publishSnapshot` factory that mirrors a row onto
 * the hub bus. (The `webContents.send` copy does not need it — Electron
 * structured-clones across the IPC boundary, so the renderer never holds a
 * reference to anything of ours.)
 */
export function detachPendingSlot(
  session: Pick<ClaudeSessionState, 'pendingApproval' | 'pendingQuestions'>,
): Pick<ClaudeSessionState, 'pendingApproval' | 'pendingQuestions'> {
  return {
    pendingApproval: detachApproval(session.pendingApproval),
    pendingQuestions: session.pendingQuestions?.map(detachQuestion) ?? null,
  };
}

function detachApproval(approval: PendingApproval | null): PendingApproval | null {
  if (!approval) return null;
  // Spread first so no key is ADDED that the original did not have — an
  // `undefined` `suggestions` is not the same wire shape as an absent one.
  const copy: PendingApproval = { ...approval, toolInput: detachToolInput(approval.toolInput) };
  if (approval.suggestions) copy.suggestions = [...approval.suggestions];
  return copy;
}

function detachQuestion(question: PendingQuestion): PendingQuestion {
  return { ...question, options: question.options?.map((o) => ({ ...o })) ?? question.options };
}

/** Tool input is arbitrary hook/daemon JSON, so it needs a real deep copy.
 *  `structuredClone` refuses functions and class instances; nothing that ever
 *  arrives here is one, but a snapshot read must not be the thing that throws,
 *  so an unexpected payload degrades to the shared reference. */
function detachToolInput(input: unknown): any {
  if (input === null || typeof input !== 'object') return input;
  try {
    return structuredClone(input);
  } catch {
    return input;
  }
}

/** A row at birth: the slot starts empty and no feed owns anything yet — the
 *  first hook or managed frame parks through its own slot. Spelled out so the
 *  two fields never have to appear in a session literal at all. */
export function bornWithEmptyPending(draft: SessionWithoutPending): PendingFencedSession {
  return bornWithPending(draft, 'hooks', undefined, () => {});
}
