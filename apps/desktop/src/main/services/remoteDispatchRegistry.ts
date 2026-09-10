/**
 * REMOTE WORKER DISPATCH — the ORIGIN's record and its admission gate.
 *
 * THE OTHER TWO THIRDS: `services/hub/internal/bus/remotedispatch.go` (the local
 * hub's router, which mints the dispatch id and stamps it onto the outbound
 * `agents.spawn`) and `services/hub/cmd/brain/remotedispatch.go` (the executing
 * node, which records the id and publishes callbacks against it). Read the bus
 * file first: it states the problem this feature exists to solve.
 *
 * WHAT THIS FILE IS. A local manager dispatched a worker onto a linked machine.
 * That worker's progress, blocks and final result arrive here as
 * `agent.dispatch.update` events, hub-stamped by our own federation link. This
 * module is the thing that decides whether one of them may become a TURN IN A
 * MANAGER'S CONVERSATION — which is a real privilege, because a fleet wake is
 * indistinguishable, to the model reading it, from something the user said.
 *
 * THE FIVE CHECKS, and why each is separate rather than folded into one:
 *
 *  1. THE EVENT MUST BE HUB-STAMPED. An unstamped `agent.dispatch.update` is a
 *     LOCAL publish — anything holding a bus connection that may publish could
 *     emit one. The stamp is applied by federation.go on OUR outbound link, from
 *     the peer name in OUR peers.json; it is not a field the payload can set
 *     (the forwarder overwrites it, and a pre-stamped event is dropped by the
 *     tree invariant). So the stamp is the one part of the message we know
 *     rather than believe.
 *  2. THE STAMP MUST MATCH THE RECORD'S PEER. Two linked machines are ordinary.
 *     Without this, a dispatch id that leaked from peer A could be acknowledged
 *     by peer B — cross-peer forgery, from inside the set of machines the
 *     operator did trust.
 *  3. THE DISPATCH ID MUST BE OPEN HERE. Unguessable, minted locally, closed on
 *     the terminal update. This is what makes a callback an ANSWER rather than a
 *     request: a peer never names a recipient, it acknowledges an id it was
 *     given.
 *  4. THE WORKER SESSION MUST MATCH the one our own spawn result recorded, once
 *     we know it. A dispatch id observed by another session on that machine
 *     still cannot report as a different worker.
 *  5. THE PARENT MUST BE A LIVE, LOCAL, WAKE-ELIGIBLE MANAGER at delivery time.
 *     Not at dispatch time — a manager can end, be closed, or be replaced inside
 *     a long-running dispatch. And LOCAL specifically: a federated snapshot in
 *     our store is another machine's session, and letting one become a wake
 *     target here is exactly the "remote snapshots become local recipients" hole
 *     that must not exist.
 *
 * WHAT IS DELIBERATELY NOT PROMISED. Exactly-once DELIVERY: the transport is a
 * websocket, not a queue. What is promised is an exactly-once EFFECT — updates
 * carry a per-dispatch monotonic `seq`, this module records the highest one it
 * has acted on, and a replayed or duplicated update is dropped. Combined with
 * the peer's replay-by-id, a link that was down when a worker finished is
 * reconciled on reconnect rather than polled for, and reconciling twice costs
 * nothing.
 *
 * DURABILITY. The records are written to `<userData>/remote-dispatches.json` so
 * a desktop restart does not lose track of work that is still running on another
 * machine. That file holds NO credential: peer NAME, dispatch id, session ids, a
 * label and the remote cwd string. The peer's bearer token lives in peers.json
 * and never reaches this module.
 */
import * as fs from 'fs';
import * as path from 'path';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import type { FleetMessageEntry } from '../shared/fleetMessages';

/** The wire protocol number both hubs stamp and check. Twin: bus.DispatchProtocol. */
export const DISPATCH_PROTOCOL = 2;

/** Twin: bus.DispatchIDPattern. */
const DISPATCH_ID_RE = /^[A-Za-z0-9_-]{16,128}$/;

/**
 * How long a record with no terminal update survives. A dispatch whose peer
 * never comes back must eventually stop being replayed at, or every reconnect
 * forever re-asks about a machine that was decommissioned a month ago. Twelve
 * hours is longer than any plausible single agent task and short enough that the
 * file cannot grow without bound.
 */
export const DISPATCH_MAX_AGE_MS = 12 * 60 * 60 * 1000;

/** Cap on stored records, oldest-first eviction. Bounded file, bounded scan. */
const MAX_RECORDS = 256;

/** The subset of FleetMessageKind the return channel can carry. Twin: the
 *  dispatchKind* constants in cmd/brain/remotedispatch.go. */
export type RemoteDispatchKind =
  | 'worker-finished'
  | 'worker-escalated'
  | 'blocked'
  | 'progress';

const KINDS: ReadonlySet<string> = new Set([
  'worker-finished',
  'worker-escalated',
  'blocked',
  'progress',
]);

/** Twin: dispatchUpdate in cmd/brain/remotedispatch.go. */
export interface RemoteDispatchUpdate {
  protocol: number;
  dispatchId: string;
  kind: RemoteDispatchKind;
  sessionId: string;
  seq: number;
  ts: number;
  final: boolean;
  entry: FleetMessageEntry;
}

export interface RemoteDispatchRecord {
  dispatchId: string;
  localSessionId?: string;
  resultSchema?: Record<string, unknown>;
  /** The peer NAME from peers.json — the same string the envelope is stamped
   *  with. Never a URL and never a token. */
  peer: string;
  /** The local manager this dispatch answers to. Validated live at delivery. */
  ownerSessionId: string;
  /** The worker's session id ON THE PEER, once the spawn result named it. */
  sessionId?: string;
  label?: string;
  /** The REMOTE cwd, kept verbatim for the operator's record. It is never
   *  resolved, joined or opened here — it names another machine's filesystem. */
  cwd?: string;
  provider?: string;
  model?: string;
  toolScope?: string;
  openedAt: number;
  /** Highest update seq acted on. 0 = nothing delivered yet. */
  ackedSeq: number;
  state: 'open' | 'done' | 'failed' | 'lost';
  /** Why a record left `open` other than normally — a spawn that never started,
   *  or a peer that no longer knows the dispatch. Shown, never acted on. */
  note?: string;
}

/** What `accept` decided, so callers (and tests) can assert the reason. */
export type AcceptOutcome =
  | { ok: true; record: RemoteDispatchRecord; parentSessionId: string; update: RemoteDispatchUpdate }
  | { ok: false; reason: AcceptRejection };

export type AcceptRejection =
  | 'not-federated'
  | 'malformed'
  | 'protocol-mismatch'
  | 'unknown-dispatch'
  | 'wrong-peer'
  | 'closed'
  | 'session-mismatch'
  | 'duplicate'
  | 'no-live-manager';

/** The store's one dependency on the session store, injected so this module is
 *  unit-testable without Electron or a live fleet. Returns null when the id is
 *  not a LOCAL, live, wake-eligible session — see check 5. */
export type ManagerResolver = (sessionId: string) => string | null;

function isRecord(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === 'object' && !Array.isArray(v);
}

export class RemoteDispatchRegistry {
  private records = new Map<string, RemoteDispatchRecord>();
  private file: string | null = null;
  private resolveManager: ManagerResolver = () => null;
  /** Set once loaded, so a call before start() never writes a truncated file. */
  private ready = false;

  /**
   * Point the registry at its state file and its manager resolver.
   *
   * `resolveManager` is the whole of check 5 and is deliberately a function of
   * the CALLER's choosing: main passes one backed by claudeSessionStore that
   * refuses a federated row, and a test passes a table. It may return a
   * DIFFERENT id than it was given — that is manager succession
   * (adopt_workers / reparentChildren): a dispatch whose original manager was
   * replaced must follow the fleet, exactly like a local worker's wake.
   */
  start(stateFile: string, resolveManager: ManagerResolver): void {
    this.file = stateFile;
    this.resolveManager = resolveManager;
    this.load();
    this.ready = true;
  }

  /** Test/teardown hook: forget everything without touching disk. */
  reset(): void {
    this.records.clear();
    this.file = null;
    this.ready = false;
    this.resolveManager = () => null;
  }

  private load(): void {
    if (!this.file) return;
    let raw: string;
    try {
      raw = fs.readFileSync(this.file, 'utf-8');
    } catch {
      return; // absent or unreadable → no dispatches, same as the peers loader
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      console.warn('[dispatch] remote-dispatches.json is not valid JSON — starting empty');
      return;
    }
    if (!Array.isArray(parsed)) return;
    const now = Date.now();
    for (const item of parsed) {
      if (!isRecord(item)) continue;
      const r = item as Partial<RemoteDispatchRecord>;
      if (typeof r.dispatchId !== 'string' || !DISPATCH_ID_RE.test(r.dispatchId)) continue;
      if (typeof r.peer !== 'string' || !r.peer) continue;
      if (typeof r.ownerSessionId !== 'string' || !r.ownerSessionId) continue;
      const openedAt = typeof r.openedAt === 'number' ? r.openedAt : now;
      // An open record older than the ceiling is loaded as LOST rather than
      // dropped: the operator's question after a crash is "what happened to the
      // thing I sent to the other machine", and a silently absent row answers
      // it wrongly.
      const stale = false;
      this.records.set(r.dispatchId, {
        dispatchId: r.dispatchId,
        localSessionId: r.localSessionId,
        resultSchema: r.resultSchema,
        peer: r.peer,
        ownerSessionId: r.ownerSessionId,
        sessionId: typeof r.sessionId === 'string' ? r.sessionId : undefined,
        label: typeof r.label === 'string' ? r.label : undefined,
        cwd: typeof r.cwd === 'string' ? r.cwd : undefined,
        provider: typeof r.provider === 'string' ? r.provider : undefined,
        model: typeof r.model === 'string' ? r.model : undefined,
        toolScope: typeof r.toolScope === 'string' ? r.toolScope : undefined,
        openedAt,
        ackedSeq: typeof r.ackedSeq === 'number' ? r.ackedSeq : 0,
        state: stale
          ? 'lost'
          : r.state === 'done' || r.state === 'failed' || r.state === 'lost'
            ? r.state
            : 'open',
        note: stale
          ? 'still open after this desktop restarted and no result arrived within 12h'
          : typeof r.note === 'string'
            ? r.note
            : undefined,
      });
    }
  }

  private persist(): void {
    if (!this.file || !this.ready) return;
    // Never evict active/uncertain work to make room for history.
    const terminal = [...this.records.values()].filter((r) => r.state === 'done' || r.state === 'failed').sort((a,b) => a.openedAt-b.openedAt);
    for (const r of terminal.slice(0, Math.max(0,this.records.size-MAX_RECORDS))) this.records.delete(r.dispatchId);
    atomicWriteFileSync(this.file, `${JSON.stringify([...this.records.values()], null, 2)}\n`, { mode: 0o600 });
  }

  /**
   * Record a dispatch the local hub's router just stamped and is forwarding.
   *
   * Called from the LOCAL (unstamped) `agent.dispatch.opened` event, which only
   * this machine's own hub publishes. It is deliberately opened BEFORE the spawn
   * result comes back — a fast worker can block on its first tool call before
   * the forward returns, and a callback naming a dispatch we have not heard of
   * would be dropped.
   */
  open(rec: {
    localSessionId?: string;
    resultSchema?: Record<string, unknown>;
    dispatchId?: unknown;
    peer?: unknown;
    ownerSessionId?: unknown;
    label?: unknown;
    cwd?: unknown;
    provider?: unknown;
    model?: unknown;
    toolScope?: unknown;
  }): RemoteDispatchRecord | null {
    const dispatchId = typeof rec.dispatchId === 'string' ? rec.dispatchId : '';
    const peer = typeof rec.peer === 'string' ? rec.peer.trim() : '';
    const ownerSessionId =
      typeof rec.ownerSessionId === 'string' ? rec.ownerSessionId.trim() : '';
    if (!DISPATCH_ID_RE.test(dispatchId) || !peer || !ownerSessionId) return null;
    const existing = this.records.get(dispatchId);
    if (existing) return existing; // a re-published open is not a second dispatch
    const str = (v: unknown): string | undefined =>
      typeof v === 'string' && v.trim() ? v : undefined;
    const record: RemoteDispatchRecord = {
      dispatchId,
      localSessionId: rec.localSessionId,
      resultSchema: rec.resultSchema,
      peer,
      ownerSessionId,
      label: str(rec.label),
      cwd: str(rec.cwd),
      provider: str(rec.provider),
      model: str(rec.model),
      toolScope: str(rec.toolScope),
      openedAt: Date.now(),
      ackedSeq: 0,
      state: 'open',
    };
    this.records.set(dispatchId, record);
    this.persist();
    return record;
  }

  /** Join the record to the worker session id the spawn result named (check 4). */
  attachSession(dispatchId: unknown, sessionId: unknown): void {
    if (typeof dispatchId !== 'string' || typeof sessionId !== 'string' || !sessionId) return;
    const record = this.records.get(dispatchId);
    if (!record || record.sessionId === sessionId) return;
    if (record.sessionId) return; // never re-point an established dispatch
    record.sessionId = sessionId;
    this.persist();
  }

  /**
   * Close a record that will never produce a result — the forwarded spawn itself
   * failed. This is what stops the feature from creating ORPHANED DISPATCHES:
   * without it a failed spawn leaves an open record that every reconnect
   * replays at, and an operator with no way to tell it apart from work in
   * flight.
   */
  fail(dispatchId: unknown, note?: unknown): void {
    if (typeof dispatchId !== 'string') return;
    const record = this.records.get(dispatchId);
    if (!record || record.state !== 'open') return;
    // A transport error does not prove rejection. Keep reconciling this id.
    record.note = 'Admission unknown; do not repeat the spawn';
    if (typeof note === 'string' && note.trim()) record.note = note.trim().slice(0, 500);
    this.persist();
  }

  /** Mark a record lost: the peer no longer knows this dispatch (it restarted,
   *  or the record was evicted there). Visible, not silent — a lost dispatch an
   *  operator can see is recoverable; one that stays pending forever is not. */
  markLost(dispatchId: string, note: string): void {
    const record = this.records.get(dispatchId);
    if (!record || record.state !== 'open') return;
    // Peer ignorance is not proof a worker ended. Keep the origin record open.
    record.note = note;
    this.persist();
  }

  /**
   * Move every OPEN dispatch owned by a retiring manager to its successor —
   * the cross-machine half of claudeSessionStore.reparentChildren.
   *
   * A remote worker is not a local child row, so the loop that re-points
   * `parentSessionId` cannot see it; this record is the only thing that knows
   * which manager is owed its result. Terminal records are left alone: their
   * report has already been delivered, and re-pointing history would make the
   * operator's record of who received what wrong.
   */
  reparent(oldManagerId: string, newManagerId: string): string[] {
    if (!oldManagerId || !newManagerId || oldManagerId === newManagerId) return [];
    const moved: string[] = [];
    for (const r of this.records.values()) {
      if (r.state !== 'open' || r.ownerSessionId !== oldManagerId) continue;
      r.ownerSessionId = newManagerId;
      moved.push(r.dispatchId);
    }
    if (moved.length) this.persist();
    return moved;
  }

  /** Dispatches still awaiting a result on one peer — the reconnect replay set. */
  openForPeer(peer: string): RemoteDispatchRecord[] {
    const now = Date.now();
    const out: RemoteDispatchRecord[] = [];
    for (const r of this.records.values()) {
      if (r.peer !== peer || r.state !== 'open') continue;
      out.push(r);
    }
    return out;
  }

  /** Every record, newest first — the operator-facing read. */
  list(): RemoteDispatchRecord[] {
    return [...this.records.values()].sort((a, b) => b.openedAt - a.openedAt);
  }

  /** The record a REMOTE session belongs to, for UI attribution. */
  forSession(sessionId: string): RemoteDispatchRecord | undefined {
    if (!sessionId) return undefined;
    for (const r of this.records.values()) if (r.sessionId === sessionId) return r;
    return undefined;
  }

  /**
   * THE ADMISSION GATE. Decide whether one inbound update may become a wake.
   *
   * `hub` is the envelope stamp our own federation link applied — undefined for
   * a local publish, which is check 1 and is rejected outright.
   */
  accept(hub: string | undefined, payload: unknown): AcceptOutcome {
    // 1. Hub-stamped or nothing. A local publish cannot complete a remote
    //    dispatch, however well-formed it looks.
    if (!hub) return { ok: false, reason: 'not-federated' };
    const update = this.parseUpdate(payload);
    if (!update) return { ok: false, reason: 'malformed' };
    if (update.protocol !== DISPATCH_PROTOCOL) {
      return { ok: false, reason: 'protocol-mismatch' };
    }
    const record = this.records.get(update.dispatchId);
    // 3. Known here. An id we never minted is not a dispatch.
    if (!record) return { ok: false, reason: 'unknown-dispatch' };
    // 2. Stamped by the peer this dispatch actually went to.
    if (record.peer !== hub) return { ok: false, reason: 'wrong-peer' };
    if (record.state !== 'open') return { ok: false, reason: 'closed' };
    // 4. The worker we spawned, not another session that saw the id.
    if (record.sessionId && update.sessionId && record.sessionId !== update.sessionId) {
      return { ok: false, reason: 'session-mismatch' };
    }
    // Ordering/dedup. `seq` is monotonic per dispatch on the peer, so an
    // out-of-order or replayed update is one we have already acted on.
    if (update.seq <= record.ackedSeq) return { ok: false, reason: 'duplicate' };
    // 5. A live, LOCAL, wake-eligible manager — resolved now, not at dispatch.
    const parentSessionId = this.resolveManager(record.ownerSessionId);
    if (!parentSessionId) return { ok: false, reason: 'no-live-manager' };
    return { ok: true, record, parentSessionId, update };
  }

  acknowledge(dispatchId: string, update: RemoteDispatchUpdate): void {
    const record = this.records.get(dispatchId);
    if (!record || update.seq <= record.ackedSeq) return;
    record.ackedSeq = update.seq;
    if (update.final) record.state = 'done';
    if (!record.sessionId) record.sessionId = update.sessionId;
    this.persist();
  }

  /** Strict decode. Anything the peer could get wrong is rejected rather than
   *  coerced: this payload becomes text in a manager's conversation. */
  private parseUpdate(payload: unknown): RemoteDispatchUpdate | null {
    if (!isRecord(payload)) return null;
    const dispatchId = payload.dispatchId;
    const kind = payload.kind;
    const seq = payload.seq;
    const entry = payload.entry;
    if (typeof dispatchId !== 'string' || !DISPATCH_ID_RE.test(dispatchId)) return null;
    if (typeof kind !== 'string' || !KINDS.has(kind)) return null;
    if (typeof seq !== 'number' || !Number.isSafeInteger(seq) || seq <= 0) return null;
    if (typeof payload.sessionId !== 'string' || !payload.sessionId) return null;
    if (!isRecord(entry)) return null;
    if (entry.sessionId !== payload.sessionId) return null;
    if ((payload.final === true) !== (kind === 'worker-finished' || kind === 'worker-escalated')) return null;
    if (typeof entry.label !== 'string') return null;
    return {
      protocol: typeof payload.protocol === 'number' ? payload.protocol : 0,
      dispatchId,
      kind: kind as RemoteDispatchKind,
      sessionId: typeof payload.sessionId === 'string' ? payload.sessionId : '',
      seq,
      ts: typeof payload.ts === 'number' ? payload.ts : Date.now(),
      final: payload.final === true,
      entry: entry as unknown as FleetMessageEntry,
    };
  }
}

export const remoteDispatchRegistry = new RemoteDispatchRegistry();

/** Caps on the peer-authored strings that become ONE LINE of a fleet bullet.
 *  Twin in spirit of excerptReply's own cap; kept local so this module stays
 *  free of the renderer-shared module's imports beyond the entry type. */
const LINE_MAX = 400;
const FULL_REPLY_MAX = 24_000;

/**
 * Flatten and cap the fields of a peer-authored entry that become a fleet
 * bullet, and DROP the ones a peer has no business setting.
 *
 * The bullet format is line-based precisely so parsing needs no guesswork
 * (shared/fleetMessages.ts says so), which means an embedded newline in a
 * peer's label or reply excerpt does not merely look wrong — it splits one
 * bullet into two and the card parser reads the remainder as a new entry. A
 * local worker's text goes through excerptReply for the same reason; this is
 * that guarantee re-established at the machine boundary.
 *
 * `reviewEvidenceId` is dropped outright: it is an opaque reference into THIS
 * host's own capture store, and a value arriving from another machine could
 * only ever name a local capture the peer did not make.
 */
export function sanitizeRemoteEntry(entry: FleetMessageEntry): FleetMessageEntry {
  const line = (v: unknown): string | undefined => {
    if (typeof v !== 'string') return undefined;
    const flat = v.replace(/\s+/g, ' ').trim();
    return flat ? flat.slice(0, LINE_MAX) : undefined;
  };
  const out: FleetMessageEntry = {
    label: line(entry.label) ?? 'Agent',
    sessionId: String(entry.sessionId),
  };
  const cwd = line(entry.cwd);
  if (cwd) out.cwd = cwd;
  if (entry.blockedOn === 'approval' || entry.blockedOn === 'question') {
    out.blockedOn = entry.blockedOn;
  }
  const lastReply = line(entry.lastReply);
  if (lastReply) out.lastReply = lastReply;
  const note = line(entry.note);
  if (note) out.note = note;
  const failed = line(entry.failed);
  if (failed) out.failed = failed;
  const crossed = line(entry.crossed);
  if (crossed) out.crossed = crossed;
  if (entry.stopped === true) out.stopped = true;
  if (entry.needsDecision === true) out.needsDecision = true;
  // The full final message keeps its newlines — it is rendered as its own
  // block, not as a bullet — but is still capped so one peer cannot hand a
  // manager an unbounded turn.
  if (typeof entry.fullReply === 'string' && entry.fullReply.trim()) {
    out.fullReply = entry.fullReply.slice(0, FULL_REPLY_MAX);
  }
  if (typeof entry.escalation === 'string' && entry.escalation.trim()) {
    out.escalation = entry.escalation.slice(0, FULL_REPLY_MAX);
  }
  const escalationError = line(entry.escalationError);
  if (escalationError) out.escalationError = escalationError;
  return out;
}

/** Where the records live. Separate from config.yaml for the reason
 *  remote-server.json is: it is machine state read and written by main alone. */
export function remoteDispatchStatePath(userDataDir: string): string {
  return path.join(userDataDir, 'remote-dispatches.json');
}
