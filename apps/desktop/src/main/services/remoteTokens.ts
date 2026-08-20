import * as crypto from 'crypto';
import * as fs from 'fs';
import * as path from 'path';
import { getConfigDir } from './configService';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { trimAsciiWhitespace } from '../lib/asciiWhitespace';
import type { RemoteTokenRecord, RemoteTokenScope } from '../shared/ipcTypes';

const VALID_SCOPES = new Set<RemoteTokenScope>(['view', 'triage', 'operator']);

function tokensPath(): string {
  return path.join(getConfigDir(), 'tokens.json');
}

function normalizeScope(scope: string): RemoteTokenScope {
  // trimAsciiWhitespace, NOT String.prototype.trim: `.trim()` strips U+FEFF (BOM)
  // and Go's authtoken.ParseScope twin does not, while Go's strips U+0085 (NEL)
  // and `.trim()` does not — so a BOM/NEL-wrapped scope minted a grant on one
  // stack and was refused on the other. Trimming the ASCII set on both makes them
  // agree (fail closed) on every non-ASCII wrapper.
  const s = trimAsciiWhitespace(scope).toLowerCase() as RemoteTokenScope;
  if (!VALID_SCOPES.has(s)) {
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

function mint(scope: RemoteTokenScope, label: string): RemoteTokenRecord {
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
  };
  writeTokens([...records, next]);
  return next;
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
