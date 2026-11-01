/**
 * Session roster policy: which sessions a session-list provider lists, and
 * which listed rows are still Starting. See docs/session-roster.md.
 *
 * This is the TypeScript twin of services/hub-rs/src/services/roster.rs. Both
 * implement the same pure rule and both load the shared vectors in
 * contracts/session-roster-cases.json, so the desktop (when it is the
 * `sessions.snapshots` provider because claudemon is external) and an engine
 * hub list exactly the same sessions. Change the rule in both places, and add
 * a vector, or the providers stop being interchangeable.
 *
 * Membership is a pure function of the row and a small context:
 *   - live rows (any mode but stopped/unknown) are always listed;
 *   - an `unknown` row is listed as Starting only when it cannot be a shell
 *     (stream transport, or a managed provider), and never when the hub knows
 *     it is a shell;
 *   - a stopped row is listed only while it is on the shared sidebar
 *     (`sessionOpen`'s open set) or in the desktop layout (curation only ever
 *     ADDS rows). There is no time window: nothing leaves on its own.
 *
 * The shared archive (`sessionArchive`) is the user's Hide. The hub removes an
 * archived id from the open set, so it needs no rule here; clients still filter
 * by the archive while that change propagates.
 */

export interface RosterContext {
  /** The shared sidebar (`sessionOpen` open ids). */
  open?: Iterable<string>;
  /** Session ids the desktop layout curates. */
  layout?: Iterable<string>;
  /** Ids the hub knows are terminal shells; never Starting. */
  shells?: Iterable<string>;
}

interface NormalizedRow {
  id: string | null;
  mode: string;
  status: string;
  transport: string;
  provider: string;
}

const has = (set: Iterable<string> | undefined, id: string | null): boolean => {
  if (!set || id === null) return false;
  if (set instanceof Set) return set.has(id);
  for (const value of set) if (value === id) return true;
  return false;
};

const optionalString = (value: unknown): value is string | null | undefined =>
  value === undefined || value === null || typeof value === 'string';

/**
 * Accepts hub-shaped snake_case rows (`session_id`) and desktop camelCase
 * snapshots (`sessionId`). Returns null when a state field has the wrong type
 * (the same fields hub-rs validates: id, mode, status, a string `updated_at`,
 * boolean `archived`): a malformed row is never listed. transport/provider are
 * advisory, and a desktop row's epoch-ms `updatedAt` is accepted.
 */
function normalize(row: unknown): NormalizedRow | null {
  if (!row || typeof row !== 'object' || Array.isArray(row)) return null;
  const r = row as Record<string, unknown>;
  const id = r.session_id ?? r.sessionId;
  for (const value of [id, r.mode, r.status, r.updated_at]) {
    if (!optionalString(value)) return null;
  }
  if (!(r.updatedAt === undefined || r.updatedAt === null || typeof r.updatedAt === 'number'))
    return null;
  if (!(r.archived === undefined || r.archived === null || typeof r.archived === 'boolean'))
    return null;
  const text = (value: unknown): string => (typeof value === 'string' ? value : '');
  return {
    id: (id as string | null | undefined) ?? null,
    mode: (r.mode as string | null | undefined) ?? '',
    status: (r.status as string | null | undefined) ?? '',
    transport: text(r.transport),
    provider: text(r.provider),
  };
}

const isStopped = (r: NormalizedRow): boolean =>
  r.mode === 'stopped' || (r.mode === '' && r.status === 'ended');

function isStarting(r: NormalizedRow, ctx: RosterContext): boolean {
  if (r.mode !== 'unknown' || has(ctx.shells, r.id)) return false;
  return r.transport === 'stream' || (r.provider !== '' && r.provider !== 'claude');
}

/** Whether a listed row should render as Starting (no first event yet). */
export function starting(row: unknown, ctx: RosterContext = {}): boolean {
  const r = normalize(row);
  return r !== null && isStarting(r, ctx);
}

/** Whether a session-list provider lists this row. There is no time window: a
 *  stopped row is listed only while it is on the sidebar (`open`) or curated
 *  in the layout, however recently it ended. */
export function listed(row: unknown, ctx: RosterContext = {}): boolean {
  const r = normalize(row);
  if (!r) return false;
  if (r.mode === 'unknown') return isStarting(r, ctx);
  if (!isStopped(r)) return true;
  return has(ctx.open, r.id) || has(ctx.layout, r.id);
}

/**
 * Session ids a layout document curates: every non-global agent's
 * `sessionId`/`lastSessionId` and its panes' `attachSessionId`. Mirrors
 * hub-rs `snapshots::layout_ids`, except that a malformed document simply
 * yields the ids that could be read (curation only adds rows now, so there is
 * no "has a layout" decision left to get wrong).
 */
export function layoutSessionIds(layout: unknown): Set<string> {
  const ids = new Set<string>();
  const agents = (layout as { data?: { agents?: unknown } } | null)?.data?.agents;
  if (!Array.isArray(agents)) return ids;
  const add = (value: unknown) => {
    if (typeof value === 'string' && value) ids.add(value);
  };
  for (const agent of agents) {
    if (!agent || typeof agent !== 'object') continue;
    const a = agent as {
      global?: unknown;
      sessionId?: unknown;
      lastSessionId?: unknown;
      tabs?: unknown;
    };
    if (a.global === true) continue;
    add(a.sessionId);
    add(a.lastSessionId);
    if (!Array.isArray(a.tabs)) continue;
    for (const tab of a.tabs) {
      const panes = (tab as { panes?: unknown } | null)?.panes;
      if (!Array.isArray(panes)) continue;
      for (const pane of panes) add((pane as { attachSessionId?: unknown } | null)?.attachSessionId);
    }
  }
  return ids;
}
