import * as crypto from 'crypto';
import * as fs from 'fs';
import * as path from 'path';
import { getConfigDir } from './configService';
import type { RemoteTokenRecord, RemoteTokenScope } from '../shared/ipcTypes';

const VALID_SCOPES = new Set<RemoteTokenScope>(['view', 'triage', 'operator']);

function tokensPath(): string {
  return path.join(getConfigDir(), 'tokens.json');
}

function normalizeScope(scope: string): RemoteTokenScope {
  const s = scope.trim().toLowerCase() as RemoteTokenScope;
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
  const file = tokensPath();
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = path.join(path.dirname(file), `.tokens-${process.pid}-${Date.now()}.json`);
  try {
    fs.writeFileSync(tmp, `${JSON.stringify(records, null, 2)}\n`, { mode: 0o600 });
    try {
      fs.chmodSync(tmp, 0o600);
    } catch {
      /* best effort on filesystems that ignore POSIX modes */
    }
    fs.renameSync(tmp, file);
  } catch (err) {
    try {
      fs.rmSync(tmp, { force: true });
    } catch {
      /* best effort cleanup */
    }
    throw err;
  }
}

function mint(scope: RemoteTokenScope, label: string): RemoteTokenRecord {
  return {
    token: crypto.randomBytes(24).toString('base64url'),
    scope,
    label: label.trim() || undefined,
    created: new Date().toISOString(),
  };
}

export function listRemoteTokens(): RemoteTokenRecord[] {
  return readTokens().sort((a, b) => b.created.localeCompare(a.created));
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
