/** Durable origin admission and result receipts. The host supplies destination
 * provenance; peer payloads never select a manager. Interrupted sends stay
 * explicitly unknown rather than being replayed as new manager work. */
import * as fs from 'fs';
import * as path from 'path';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import type { FleetMessageEntry } from '../shared/fleetMessages';

/** The wire protocol number both hubs stamp and check. Twin: bus.DispatchProtocol. */
export const DISPATCH_PROTOCOL = 2;

/** Twin: bus.DispatchIDPattern. */
const DISPATCH_ID_RE = /^[A-Za-z0-9_-]{16,128}$/;

/** Cap on stored records, oldest-first eviction. Bounded file, bounded scan. */
const MAX_RECORDS = 256;

/** The subset of FleetMessageKind the return channel can carry. Twin: the
 *  dispatchKind* constants in cmd/brain/remotedispatch.go. */
export type RemoteDispatchKind = 'worker-finished' | 'worker-escalated' | 'blocked' | 'progress';

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
  deliveringSeq?: number;
  /** Authenticated peer evidence, retained locally before task/wake delivery. */
  lastUpdate?: RemoteDispatchUpdate;
  state: 'open' | 'done' | 'failed' | 'lost';
  /** Why a record left `open` other than normally — a spawn that never started,
   *  or a peer that no longer knows the dispatch. Shown, never acted on. */
  note?: string;
}

/** What `accept` decided, so callers (and tests) can assert the reason. */
export type AcceptOutcome =
  | {
      ok: true;
      record: RemoteDispatchRecord;
      parentSessionId: string;
      update: RemoteDispatchUpdate;
    }
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
  | 'no-live-manager'
  | 'delivery-unknown';

/** The store's one dependency on the session store, injected so this module is
 *  unit-testable without Electron or a live fleet. Returns null when the id is
 *  not a LOCAL, live, wake-eligible session — see check 5. */
export type ManagerResolver = (sessionId: string) => string | null;

function isRecord(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === 'object' && !Array.isArray(v);
}

export class RemoteDispatchRegistry {
  /** Adoption is also a reconciliation edge: a result may have arrived while
   * its old manager was unavailable, without any network disconnect. */
  onReparent: () => void = () => {};
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
    this.records.clear();
    this.load();
    this.ready = true;
  }

  /** Test/teardown hook: forget everything without touching disk. */
  reset(): void {
    this.onReparent = () => {};
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
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return;
      throw new Error('Remote dispatch journal is unreadable; dispatch disabled');
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      throw new Error('Remote dispatch journal is invalid; dispatch disabled');
    }
    if (!Array.isArray(parsed)) throw new Error('Remote dispatch journal has invalid shape');
    const now = Date.now();
    for (const item of parsed) {
      if (!isRecord(item)) throw new Error('Invalid remote dispatch record');
      const r = item as Partial<RemoteDispatchRecord>;
      if (typeof r.dispatchId !== 'string' || !DISPATCH_ID_RE.test(r.dispatchId))
        throw new Error('Invalid remote dispatch id');
      if (typeof r.peer !== 'string' || !r.peer) throw new Error('Invalid remote destination');
      if (typeof r.ownerSessionId !== 'string' || !r.ownerSessionId)
        throw new Error('Invalid remote dispatch owner');
      const openedAt = typeof r.openedAt === 'number' ? r.openedAt : now;
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
        deliveringSeq: r.deliveringSeq,
        lastUpdate: r.lastUpdate,
        ackedSeq: typeof r.ackedSeq === 'number' ? r.ackedSeq : 0,
        state: r.state === 'done' || r.state === 'failed' ? r.state : 'open',
        note: typeof r.note === 'string' ? r.note : undefined,
      });
    }
  }

  private persist(): void {
    if (!this.file || !this.ready) return;
    // Never evict active/uncertain work to make room for history.
    const terminal = [...this.records.values()]
      .filter((r) => r.state === 'done' || r.state === 'failed')
      .sort((a, b) => a.openedAt - b.openedAt);
    for (const r of terminal.slice(0, Math.max(0, this.records.size - MAX_RECORDS)))
      this.records.delete(r.dispatchId);
    atomicWriteFileSync(this.file, `${JSON.stringify([...this.records.values()], null, 2)}\n`, {
      mode: 0o600,
    });
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
    const ownerSessionId = typeof rec.ownerSessionId === 'string' ? rec.ownerSessionId.trim() : '';
    if (!DISPATCH_ID_RE.test(dispatchId) || !peer || !ownerSessionId) return null;
    const existing = this.records.get(dispatchId);
    if (existing) return existing; // a re-published open is not a second dispatch
    if ([...this.records.values()].filter((r) => r.state === 'open').length >= MAX_RECORDS)
      throw new Error('Unresolved remote dispatch limit reached; no worker started');
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

  /** Retain uncertain admission for reconciliation; transport failure is not rejection. */
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
    // Remote journal loss cannot erase a locally retained terminal outcome.
    // An open receipt means delivery is unconfirmed, not that the outcome is
    // unknown. Nonterminal evidence still cannot establish an outcome.
    const projectedNote = record.lastUpdate?.final === true
      ? 'The remote server no longer has a record of this dispatch. The terminal outcome is retained locally; manager wake delivery is unconfirmed. Reconcile delivery without repeating the spawn or wake.'
      : note;
    if (record.note === projectedNote) return;
    // Peer ignorance is not proof a worker ended. Keep the origin record open.
    const previous = record.note;
    record.note = projectedNote;
    try {
      this.persist();
    } catch (error) {
      // A failed write must not suppress a later reconnect's persistence retry.
      if (previous === undefined) delete record.note;
      else record.note = previous;
      throw error;
    }
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
    if (moved.length) {
      this.persist();
      this.onReparent();
    }
    return moved;
  }

  /** Dispatches still awaiting a result on one peer — the reconnect replay set. */
  openForPeer(peer: string): RemoteDispatchRecord[] {
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
    if (record.deliveringSeq !== undefined) return { ok: false, reason: 'delivery-unknown' };
    // 5. A live, LOCAL, wake-eligible manager — resolved now, not at dispatch.
    const parentSessionId = this.resolveManager(record.ownerSessionId);
    if (!parentSessionId) return { ok: false, reason: 'no-live-manager' };
    return { ok: true, record, parentSessionId, update };
  }

  retainEvidence(dispatchId: string, update: RemoteDispatchUpdate): void {
    const record = this.records.get(dispatchId);
    if (!record) throw new Error('Unknown dispatch');
    record.lastUpdate = { ...update, entry: sanitizeRemoteEntry(update.entry) };
    this.persist();
  }

  beginDelivery(dispatchId: string, seq: number): void {
    const record = this.records.get(dispatchId);
    if (!record) throw new Error('Unknown dispatch');
    record.deliveringSeq = seq;
    record.note =
      'Wake delivery in progress; after interruption delivery is unknown, never resend blindly';
    this.persist();
  }

  acknowledge(dispatchId: string, update: RemoteDispatchUpdate): void {
    const record = this.records.get(dispatchId);
    if (!record || update.seq <= record.ackedSeq) return;
    record.ackedSeq = update.seq;
    delete record.deliveringSeq;
    delete record.note;
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
    if ((payload.final === true) !== (kind === 'worker-finished' || kind === 'worker-escalated'))
      return null;
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
