import * as crypto from 'crypto';
import * as fs from 'fs';
import * as path from 'path';
import { getConfigDir } from './configService';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { trimAsciiWhitespace } from '../lib/asciiWhitespace';
import type { PairingScope, RemoteTokenRecord, RemoteTokenScope } from '../shared/ipcTypes';

/** Every scope that may appear in tokens.json — the RECORD-VALIDITY question.
 *
 *  `provider` is here and not in MINTABLE_SCOPES below, and the difference is
 *  load-bearing rather than pedantic. This file and the Go store SHARE
 *  tokens.json: normalizeRecord returns null for a record whose scope is not in
 *  this set, readTokens filters those nulls out, and mint writes
 *  `[...readTokens(), next]` back to disk. So on any machine where the desktop
 *  and the hub share a config dir, a scope missing from THIS set means the next
 *  token the desktop mints silently DELETES the record — and for a provider
 *  record that is a remote node's only credential: it 401s on its next
 *  reconnect, with nothing anywhere saying why.
 *  TWIN: authtoken.ParseScope (services/hub/internal/authtoken). */
const VALID_SCOPES = new Set<RemoteTokenScope>(['view', 'triage', 'operator', 'provider']);

/** The scopes the desktop may MINT — the pairing question, deliberately
 *  narrower. A provider token's whole point is its `provides` register grant,
 *  which `workspacer token create --scope provider` fills in on the hub; minting
 *  one here would write a record with no grant that registers nothing and looks
 *  like it works. See PairingScope (ipcTypes.ts). */
const MINTABLE_SCOPES = new Set<PairingScope>(['view', 'triage', 'operator']);

function tokensPath(): string {
  return path.join(getConfigDir(), 'tokens.json');
}

/** Normalize a scope the desktop is about to MINT. Rejects `provider` — see
 *  MINTABLE_SCOPES. Reading an existing provider record goes through
 *  normalizeRecord/VALID_SCOPES instead and is unaffected. */
function normalizeScope(scope: string): PairingScope {
  // trimAsciiWhitespace, NOT String.prototype.trim: `.trim()` strips U+FEFF (BOM)
  // and Go's authtoken.ParseScope twin does not, while Go's strips U+0085 (NEL)
  // and `.trim()` does not — so a BOM/NEL-wrapped scope minted a grant on one
  // stack and was refused on the other. Trimming the ASCII set on both makes them
  // agree (fail closed) on every non-ASCII wrapper.
  const s = trimAsciiWhitespace(scope).toLowerCase() as PairingScope;
  if (!MINTABLE_SCOPES.has(s)) {
    throw new Error(`unknown remote token scope "${scope}"`);
  }
  return s;
}

function normalizeRecord(raw: unknown): RemoteTokenRecord | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Partial<RemoteTokenRecord>;
  if (typeof r.token !== 'string' || !r.token.trim()) return null;
  if (typeof r.scope !== 'string' || !VALID_SCOPES.has(r.scope as RemoteTokenScope)) return null;
  return {
    token: r.token,
    scope: r.scope as RemoteTokenScope,
    label: typeof r.label === 'string' ? r.label : undefined,
    created: typeof r.created === 'string' ? r.created : new Date(0).toISOString(),
    // Preserve, or a rewrite of tokens.json would silently strip every
    // session token's plugin grants.
    ...(Array.isArray(r.plugins) && {
      plugins: r.plugins.filter((p): p is string => typeof p === 'string' && !!p.trim()),
    }),
    // Same preservation rule for the fleet-manager profile grant.
    ...(Array.isArray(r.profilesAllowed) && {
      profilesAllowed: r.profilesAllowed.filter(
        (p): p is string => typeof p === 'string' && !!p.trim(),
      ),
    }),
    // …and the full-access grant (only the true case is ever stored).
    ...(r.yoloAllowed === true && { yoloAllowed: true as const }),
    // …and the session-role tag the grant reconciler keys on.
    ...(r.role === 'manager' && { role: r.role }),
    // …and the provider tier's REGISTER grant. Same preservation rule as the
    // four above, and the one with the sharpest failure: this record belongs to
    // a remote node, the desktop never writes it, and readTokens→mint rewrites
    // the whole file — so without this clause the next token minted here hands
    // the node's record back to disk with its grant stripped. The node then
    // reconnects, registers nothing, and re-sends `register` every 5 seconds
    // forever, because the hub's ack cannot say why a method was withheld.
    // TWIN: authtoken.Record.Provides.
    ...(Array.isArray(r.provides) && {
      provides: r.provides.filter((p): p is string => typeof p === 'string' && !!p.trim()),
    }),
  };
}

function readTokens(): RemoteTokenRecord[] {
  const file = tokensPath();
  try {
    const parsed = JSON.parse(fs.readFileSync(file, 'utf-8')) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.map(normalizeRecord).filter((r): r is RemoteTokenRecord => !!r);
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return [];
    throw err;
  }
}

function writeTokens(records: RemoteTokenRecord[]): void {
  // Secrets file: atomic write (temp + rename) with a restrictive 0o600 mode so
  // a crash can't leave a truncated token store and the file is never
  // world-readable. Shared impl in atomicWriteFile.ts.
  atomicWriteFileSync(tokensPath(), `${JSON.stringify(records, null, 2)}\n`, { mode: 0o600 });
}

function mint(scope: PairingScope, label: string): RemoteTokenRecord {
  return {
    token: crypto.randomBytes(24).toString('base64url'),
    scope,
    label: label.trim() || undefined,
    created: new Date().toISOString(),
  };
}

/** Label prefix marking a per-session MCP-facade token. Session tokens live in
 *  the same tokens.json as remote-pairing tokens (the facade and the hub read
 *  one file), so the prefix is what separates the two lifecycles: session
 *  tokens are minted at spawn, revoked when the session is evicted, and swept
 *  at boot against the live session list. */
const SESSION_LABEL_PREFIX = 'session:';

function isSessionToken(r: RemoteTokenRecord): boolean {
  return !!r.label?.startsWith(SESSION_LABEL_PREFIX);
}

export function listRemoteTokens(): RemoteTokenRecord[] {
  // Session facade tokens are lifecycle-managed plumbing, not user pairings —
  // keep them out of the Remote Control settings UI.
  return readTokens()
    .filter((r) => !isSessionToken(r))
    .sort((a, b) => b.created.localeCompare(a.created));
}

/**
 * Mint (or replace) the MCP-facade token for a session. One token per session:
 * a respawn onto the same id re-mints, so the old record never lingers with a
 * stale scope or plugin list.
 */
export function mintSessionFacadeToken(
  sessionId: string,
  scope: RemoteTokenScope,
  plugins?: string[],
  profilesAllowed?: string[],
  yoloAllowed?: boolean,
  role?: SessionTokenRole,
): RemoteTokenRecord {
  const label = SESSION_LABEL_PREFIX + sessionId;
  const records = readTokens().filter((r) => r.label !== label);
  const next: RemoteTokenRecord = {
    ...mint(normalizeScope(scope), label),
    ...(plugins && plugins.length && { plugins }),
    // Fleet-manager dispatch grant: exact profile ids this session may spawn
    // workers under. Omitted when empty (wire-shape twin of `plugins`; pinned
    // Go-side by TestProfilesAllowedWireShape).
    ...(profilesAllowed && profilesAllowed.length && { profilesAllowed }),
    // Fleet-manager full-access grant: omitted when false, same omitempty
    // wire shape (TWIN: authtoken.Record.YoloAllowed).
    ...(yoloAllowed && { yoloAllowed: true as const }),
    // Session-role tag so a later config flip can find and re-grant/revoke
    // exactly this kind of token (see reconcileSessionFacadeGrants).
    ...(role && { role }),
  };
  writeTokens([...records, next]);
  return next;
}

/** Session roles whose tokens carry a config-governed full-access grant. */
export type SessionTokenRole = 'manager';

/**
 * Bring every role-tagged session token's full-access grant in line with the
 * config that governs it (manager: agents.fleetFullAccess / per-project yolo —
 * resolved by the caller, this just applies). The MCP facade re-reads the token
 * record per request, so this IS the live apply: flipping a flag re-grants or
 * REVOKES a running manager's dispatch bypass immediately, no respawn. Only
 * session tokens
 * with a role are touched — remote pairings and plain facade workers never
 * carried the grant vocabulary and are left alone. Returns one entry per record
 * that actually CHANGED — a live session whose dispatch bypass just appeared or
 * vanished, which is exactly the set worth telling the user about (see
 * fullAccessGrants' announce step); an empty array means the flip touched
 * nothing live.
 */
export interface SessionGrantFlip {
  sessionId: string;
  role: SessionTokenRole;
  /** The grant the record now holds. */
  yoloAllowed: boolean;
}

export function reconcileSessionFacadeGrants(
  desired: Record<SessionTokenRole, boolean>,
): SessionGrantFlip[] {
  const records = readTokens();
  const flips: SessionGrantFlip[] = [];
  const next = records.map((r) => {
    if (!isSessionToken(r) || !r.role) return r;
    const want = desired[r.role];
    if (want === (r.yoloAllowed === true)) return r;
    flips.push({
      // isSessionToken already proved the prefix, but `label` is optional on
      // the record type — read it defensively rather than assert.
      sessionId: (r.label ?? '').slice(SESSION_LABEL_PREFIX.length),
      role: r.role,
      yoloAllowed: want,
    });
    const { yoloAllowed: _dropped, ...rest } = r;
    return want ? { ...rest, yoloAllowed: true as const } : rest;
  });
  if (flips.length) writeTokens(next);
  return flips;
}

/**
 * Reconcile ONE session's facade token: stamp its role (tokens minted before
 * roles existed have none — this is how a reused live manager adopts the tag)
 * and set its full-access grant to `yoloAllowed`. Returns whether the record
 * changed. No-op when the session has no token.
 */
export function reconcileSessionFacadeToken(
  sessionId: string,
  role: SessionTokenRole,
  yoloAllowed: boolean,
): boolean {
  const label = SESSION_LABEL_PREFIX + sessionId;
  const records = readTokens();
  const idx = records.findIndex((r) => r.label === label);
  if (idx < 0) return false;
  const r = records[idx];
  if (r.role === role && (r.yoloAllowed === true) === yoloAllowed) return false;
  const { yoloAllowed: _dropped, ...rest } = r;
  records[idx] = { ...rest, role, ...(yoloAllowed && { yoloAllowed: true as const }) };
  writeTokens(records);
  return true;
}

/** Revoke a session's facade token(s). No-op when none exist. */
export function revokeSessionFacadeTokens(sessionId: string): void {
  const label = SESSION_LABEL_PREFIX + sessionId;
  const records = readTokens();
  const kept = records.filter((r) => r.label !== label);
  if (kept.length !== records.length) writeTokens(kept);
}

/**
 * Drop session facade tokens whose session is no longer alive. Called at boot
 * with the daemon's live session list — sessions outlive desktop restarts, so
 * "revoke everything" would cut running agents off mid-task, and "revoke
 * nothing" would let tokens for long-gone sessions accumulate as live bearer
 * secrets.
 */
export function sweepSessionFacadeTokens(liveSessionIds: Iterable<string>): number {
  const live = new Set<string>();
  for (const id of liveSessionIds) live.add(SESSION_LABEL_PREFIX + id);
  const records = readTokens();
  const kept = records.filter((r) => !isSessionToken(r) || live.has(r.label!));
  if (kept.length === records.length) return 0;
  writeTokens(kept);
  return records.length - kept.length;
}

export function getOrCreateRemoteToken(scopeInput: string, labelInput?: string): RemoteTokenRecord {
  const scope = normalizeScope(scopeInput);
  const label = (labelInput?.trim() || `Remote Control: ${scope}`) as string;
  const records = readTokens();
  const existing = records.find((r) => r.scope === scope && r.label === label);
  if (existing) return existing;
  const next = mint(scope, label);
  writeTokens([...records, next]);
  return next;
}

export function revokeRemoteToken(token: string): RemoteTokenRecord {
  const ref = token.trim();
  if (!ref) throw new Error('missing token');
  const records = readTokens();
  const idx = records.findIndex((r) => r.token === ref);
  if (idx < 0) throw new Error('token not found');
  const [removed] = records.splice(idx, 1);
  writeTokens(records);
  return removed;
}
