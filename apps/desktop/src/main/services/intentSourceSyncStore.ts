import { createHash } from 'node:crypto';
import type { DatabaseSync } from 'node:sqlite';
import type {
  IntentExternalState,
  IntentSource,
  IntentSourceArtifact,
  IntentSourceConnection,
} from '../shared/intentSources';
import { SourceHttpError, type IntentSourceAdapter } from './intentSourceAdapters';
import type { SourceSyncResult } from './intentSourceSync';

export const INTENT_SOURCE_SYNC_SCHEMA = `
CREATE TABLE IF NOT EXISTS intent_external_objects (
 source_id TEXT PRIMARY KEY REFERENCES intent_sources(id), next_attempt INTEGER NOT NULL, snapshot TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS intent_external_due ON intent_external_objects(next_attempt);
CREATE TABLE IF NOT EXISTS intent_source_artifacts (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, source_id TEXT NOT NULL REFERENCES intent_sources(id),
 digest TEXT NOT NULL, observed_at TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(source_id, digest)
);
CREATE TABLE IF NOT EXISTS intent_source_events (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, source_id TEXT NOT NULL REFERENCES intent_sources(id),
 artifact_digest TEXT NOT NULL, observed_at TEXT NOT NULL, status TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS intent_source_events_source ON intent_source_events(source_id,sequence);
CREATE TABLE IF NOT EXISTS intent_source_accounts (id TEXT PRIMARY KEY, available_at INTEGER NOT NULL);
`;
const PERIOD = 5 * 60_000;
const RECONCILE = 30 * 60_000;
const hash = (text: string) => createHash('sha256').update(text).digest('hex');

/** One account lease shared by import, manual refresh and background sync, including across hosts
 * sharing this SQLite DB. No token is used as a key. A crashed request releases after two minutes. */
class AccountBusy extends Error {
  constructor(readonly until: number) {
    super('Source account is busy or backing off; retry after its next scheduled attempt.');
  }
}
export class IntentSourceSyncStore {
  constructor(
    private db: DatabaseSync,
    private adapter: IntentSourceAdapter,
    private now: () => number = Date.now,
    private random: () => number = Math.random,
  ) {}
  state(id: string): IntentExternalState | undefined {
    const row = this.db
      .prepare('SELECT snapshot FROM intent_external_objects WHERE source_id=?')
      .get(id);
    return row ? JSON.parse(String(row.snapshot)) : undefined;
  }
  private account(c: IntentSourceConnection): string {
    const u = new URL(c.url);
    return hash(
      JSON.stringify([
        c.provider,
        u.host,
        c.provider === 'ado' ? u.pathname.split('/')[1].toLowerCase() : '',
      ]),
    );
  }
  private claim(c: IntentSourceConnection): string {
    const id = this.account(c);
    const now = this.now();
    const row = this.db
      .prepare(
        `INSERT INTO intent_source_accounts VALUES(?,?)
      ON CONFLICT(id) DO UPDATE SET available_at=excluded.available_at WHERE available_at<=?
      RETURNING id`,
      )
      .get(id, now + 120_000, now);
    if (!row)
      throw new AccountBusy(
        Number(
          this.db.prepare('SELECT available_at FROM intent_source_accounts WHERE id=?').get(id)
            ?.available_at ?? now + 5000,
        ),
      );
    return id;
  }
  private release(account: string, after = 5000) {
    this.db
      .prepare('UPDATE intent_source_accounts SET available_at=? WHERE id=?')
      .run(this.now() + after, account);
  }
  private async read(c: IntentSourceConnection, etag?: string): Promise<SourceSyncResult> {
    if (this.adapter.sync) return this.adapter.sync(c, etag);
    const read = await this.adapter.read(c);
    return {
      ...read,
      projection: {
        objectType: c.provider === 'jira' ? 'issue' : 'work-item',
        nativeId: read.nativeId,
        url: c.url,
        revision: read.snapshot.revision,
        state: '',
        summary: {},
      },
      payload: { native: read.snapshot.fields },
    };
  }
  async import(c: IntentSourceConnection): Promise<SourceSyncResult> {
    const account = this.claim(c);
    try {
      const read = await this.read(c);
      this.release(account);
      return read;
    } catch (error) {
      this.release(
        account,
        error instanceof SourceHttpError ? Math.max(30_000, error.retryAfter) : 30_000,
      );
      throw error;
    }
  }
  /** Called inside the caller's transaction. Accepted snapshots are never mutated here. */
  record(sourceId: string, read: SourceSyncResult): IntentExternalState {
    const old = this.state(sourceId);
    const now = this.now();
    const at = new Date(now).toISOString();
    let digest = old?.artifactDigest;
    if (read.payload) {
      // Canonical JSON digest ignores dictionary order and volatile local observation time.
      const stable = (v: unknown): unknown =>
        Array.isArray(v)
          ? v.map(stable)
          : v && typeof v === 'object'
            ? Object.fromEntries(
                Object.entries(v)
                  .sort(([a], [b]) => a.localeCompare(b))
                  .map(([k, value]) => [k, stable(value)]),
              )
            : v;
      const payload = JSON.stringify(stable(read.payload));
      if (Buffer.byteLength(payload) > 2 * 1024 * 1024)
        throw new Error('Source artifact exceeds limit');
      digest = hash(payload);
      this.db
        .prepare(
          'INSERT OR IGNORE INTO intent_source_artifacts(source_id,digest,observed_at,payload) VALUES(?,?,?,?)',
        )
        .run(sourceId, digest, at, payload);
    }
    const state: IntentExternalState = {
      projection: read.projection ?? old?.projection ?? null,
      observedAt: at,
      lastSuccess: at,
      lastFailure: old?.lastFailure ?? null,
      freshnessUntil: new Date(now + PERIOD * 2).toISOString(),
      nextAttempt: now + PERIOD,
      failures: 0,
      status: read.partial ? 'partial' : 'fresh',
      detail: read.partial
        ? 'Bounded or unavailable collections; inspect artifact coverage.'
        : 'Authoritative provider observation.',
      etag: read.notModified ? old?.etag : read.etag,
      reconciledAt: read.notModified ? old?.reconciledAt : at,
      artifactDigest: digest,
    };
    if (digest && (!old || old.artifactDigest !== digest || old.status !== state.status))
      this.event(sourceId, digest, at, state.status);
    this.save(sourceId, state);
    return state;
  }
  private event(id: string, digest: string, at: string, status: string) {
    this.db
      .prepare(
        'INSERT INTO intent_source_events(source_id,artifact_digest,observed_at,status) VALUES(?,?,?,?)',
      )
      .run(id, digest, at, status);
  }
  private save(id: string, state: IntentExternalState) {
    this.db
      .prepare(
        'INSERT INTO intent_external_objects VALUES(?,?,?) ON CONFLICT(source_id) DO UPDATE SET next_attempt=excluded.next_attempt,snapshot=excluded.snapshot',
      )
      .run(id, state.nextAttempt, JSON.stringify(state));
  }
  async refresh(source: IntentSource, apply: (read: SourceSyncResult) => void): Promise<void> {
    const old = this.state(source.id);
    let account: string;
    try {
      account = this.claim(source);
    } catch (error) {
      if (error instanceof AccountBusy) {
        const at = new Date(this.now()).toISOString();
        // Move blocked rows out of the bounded due window; otherwise a large account
        // can starve unrelated accounts after a rate limit or a host restart.
        this.save(source.id, {
          projection: null,
          observedAt: at,
          lastSuccess: null,
          lastFailure: null,
          freshnessUntil: at,
          failures: 0,
          status: 'error',
          detail: 'Waiting for source account cooldown.',
          ...old,
          nextAttempt: error.until,
        });
      }
      throw error;
    }
    let cooldown = 5000;
    try {
      const conditional =
        old?.status === 'fresh' && this.now() - Date.parse(old.reconciledAt || '') < RECONCILE
          ? old.etag
          : undefined;
      const read = await this.read(source, conditional);
      if (read.notModified && !old?.projection)
        throw new Error('Conditional response without projection');
      this.db.exec('BEGIN IMMEDIATE');
      try {
        this.record(source.id, read);
        apply(read);
        this.db.exec('COMMIT');
      } catch (error) {
        this.db.exec('ROLLBACK');
        throw error;
      }
    } catch (error) {
      const status = error instanceof SourceHttpError ? error.status : 0;
      const failures = Math.min(20, (old?.failures ?? 0) + 1);
      cooldown = Math.max(
        error instanceof SourceHttpError ? error.retryAfter : 0,
        Math.min(3600_000, 30_000 * 2 ** (failures - 1)) * (0.75 + this.random() * 0.5),
      );
      const now = this.now(),
        at = new Date(now).toISOString();
      const missing = status === 404 || status === 410;
      const state: IntentExternalState = {
        projection: old?.projection ?? null,
        observedAt: at,
        lastSuccess: old?.lastSuccess ?? null,
        lastFailure: at,
        freshnessUntil: old?.freshnessUntil ?? at,
        nextAttempt: now + cooldown,
        failures,
        status: missing ? 'missing' : status === 429 ? 'rate-limited' : 'error',
        detail: missing
          ? 'Object deleted or inaccessible; last successful projection retained.'
          : status
            ? `Provider returned HTTP ${status}; retry scheduled.`
            : 'Provider synchronization failed; retry scheduled.',
        artifactDigest: old?.artifactDigest,
        reconciledAt: old?.reconciledAt,
      };
      // Failure observations are immutable too; a repeated failure adds no duplicate event.
      // A 404 remains ambiguous because permissions may hide existence.
      if (old?.status !== state.status) {
        const payload = JSON.stringify({
          ...(missing ? { tombstone: 'deleted-or-inaccessible' } : { failure: state.status }),
          status,
          previousArtifact: old?.artifactDigest,
        });
        const digest = hash(payload);
        this.db
          .prepare(
            'INSERT OR IGNORE INTO intent_source_artifacts(source_id,digest,observed_at,payload) VALUES(?,?,?,?)',
          )
          .run(source.id, digest, at, payload);
        this.event(source.id, digest, at, state.status);
      }
      this.save(source.id, state);
    } finally {
      this.release(account, cooldown);
    }
  }
  artifacts(id: string, before?: number): IntentSourceArtifact[] {
    return this.db
      .prepare(
        `SELECT e.sequence,e.observed_at,e.status,a.digest,a.payload
      FROM intent_source_events e JOIN intent_source_artifacts a
      ON a.source_id=e.source_id AND a.digest=e.artifact_digest
      WHERE e.source_id=? AND e.sequence<? ORDER BY e.sequence DESC LIMIT 1`,
      )
      .all(id, before ?? Number.MAX_SAFE_INTEGER)
      .map((r) => ({
        sequence: Number(r.sequence),
        digest: String(r.digest),
        observedAt: String(r.observed_at),
        eventStatus: String(r.status),
        payload: JSON.parse(String(r.payload)),
      }));
  }
  due(): Array<{ id: string; workspaceId: string }> {
    return this.db
      .prepare(
        `SELECT s.id, s.workspace_id FROM intent_sources s
      LEFT JOIN intent_external_objects e ON e.source_id=s.id
      WHERE json_extract(s.snapshot,'$.provider') IN ('ado','jira') AND COALESCE(e.next_attempt,0)<=?
      ORDER BY COALESCE(e.next_attempt,0),s.rowid LIMIT 32`,
      )
      .all(this.now())
      .map((r) => ({ id: String(r.id), workspaceId: String(r.workspace_id) }));
  }
}
