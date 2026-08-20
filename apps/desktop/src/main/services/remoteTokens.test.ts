import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const h = vi.hoisted(() => ({ dir: '' }));
vi.mock('./configService', () => ({ getConfigDir: () => h.dir }));

import {
  getOrCreateRemoteToken,
  listRemoteTokens,
  revokeRemoteToken,
  mintSessionFacadeToken,
  revokeSessionFacadeTokens,
  sweepSessionFacadeTokens,
} from './remoteTokens';

function tokensFile(): string {
  return path.join(h.dir, 'tokens.json');
}

beforeEach(() => {
  h.dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-remote-tokens-'));
});

afterEach(() => {
  fs.rmSync(h.dir, { recursive: true, force: true });
});

describe('remoteTokens', () => {
  it('creates a scoped hub token record and reuses scope+label pairings', () => {
    const first = getOrCreateRemoteToken('triage', 'Phone pairing');
    const second = getOrCreateRemoteToken('triage', 'Phone pairing');

    expect(second).toEqual(first);
    expect(first).toMatchObject({
      scope: 'triage',
      label: 'Phone pairing',
      created: expect.any(String),
    });
    expect(first.token).toMatch(/^[A-Za-z0-9_-]{32}$/);

    const raw = JSON.parse(fs.readFileSync(tokensFile(), 'utf-8'));
    expect(raw).toEqual([first]);
    expect(listRemoteTokens()).toEqual([first]);
  });

  it('keeps separate records for different scopes', () => {
    const triage = getOrCreateRemoteToken('triage', 'Phone pairing');
    const view = getOrCreateRemoteToken('view', 'Phone pairing');
    const operator = getOrCreateRemoteToken('operator', 'Full control pairing');

    expect(new Set([triage.token, view.token, operator.token]).size).toBe(3);
    expect(
      listRemoteTokens()
        .map((r) => r.scope)
        .sort(),
    ).toEqual(['operator', 'triage', 'view']);
  });

  it('rejects unknown scopes before writing a token file', () => {
    expect(() => getOrCreateRemoteToken('admin', 'Bad pairing')).toThrow(
      /unknown remote token scope/,
    );
    expect(fs.existsSync(tokensFile())).toBe(false);
  });

  it('trims only ASCII whitespace from a scope, agreeing with the Go ParseScope twin', () => {
    // ASCII wrappers are trimmed on both stacks.
    expect(getOrCreateRemoteToken('  operator  ', 'spaced').scope).toBe('operator');
    expect(getOrCreateRemoteToken('\t\nview\n\t', 'whitespaced').scope).toBe('view');

    // BOM (U+FEFF) and NEL (U+0085) are the two code points String.prototype.trim
    // and Go's strings.TrimSpace disagree on. A BOM-wrapped scope must be REJECTED
    // here just as authtoken.ParseScope rejects it — a plain `.trim()` would strip
    // the BOM and silently mint an operator token the Go twin refuses.
    expect(() => getOrCreateRemoteToken('﻿operator', 'bom')).toThrow(/unknown remote token scope/);
    expect(() => getOrCreateRemoteToken('operator﻿', 'bom-trail')).toThrow(
      /unknown remote token scope/,
    );
    expect(() => getOrCreateRemoteToken('operator', 'nel')).toThrow(/unknown remote token scope/);
    expect(() => getOrCreateRemoteToken(' operator', 'nbsp')).toThrow(/unknown remote token scope/);
  });

  it('revokes a token from the shared hub token file', () => {
    const keep = getOrCreateRemoteToken('view', 'Dashboard');
    const remove = getOrCreateRemoteToken('triage', 'Phone pairing');

    expect(revokeRemoteToken(remove.token)).toEqual(remove);
    expect(listRemoteTokens()).toEqual([keep]);
    expect(JSON.parse(fs.readFileSync(tokensFile(), 'utf-8'))).toEqual([keep]);
    expect(() => revokeRemoteToken(remove.token)).toThrow(/token not found/);
  });
});

describe('session facade tokens', () => {
  const rawTokens = () =>
    JSON.parse(fs.readFileSync(tokensFile(), 'utf-8')) as Array<Record<string, unknown>>;

  it('mints a session-labelled record with scope + plugins', () => {
    const rec = mintSessionFacadeToken('sess-1', 'view', ['djtouchette.jira']);

    expect(rec).toMatchObject({
      scope: 'view',
      label: 'session:sess-1',
      plugins: ['djtouchette.jira'],
    });
    expect(rawTokens()).toEqual([rec]);
  });

  it('records a profilesAllowed grant, omits it when empty, and preserves it across rewrites', () => {
    // Wire-shape TWIN of TestProfilesAllowedWireShape (services/hub
    // internal/authtoken): present as an exact-id array, ABSENT when empty —
    // never null/[] — so the Go side's omitempty round-trips.
    const rec = mintSessionFacadeToken('sess-mgr', 'operator', undefined, ['default', 'work']);
    expect(rec.profilesAllowed).toEqual(['default', 'work']);
    const raw = rawTokens().find((r) => r.token === rec.token)!;
    expect(raw.profilesAllowed).toEqual(['default', 'work']);
    expect('plugins' in raw).toBe(false);

    // A pairing-list write cycle must not strip the grant (readTokens
    // normalization preserves it like `plugins`).
    getOrCreateRemoteToken('view', 'Dashboard');
    const kept = rawTokens().find((r) => r.token === rec.token)!;
    expect(kept.profilesAllowed).toEqual(['default', 'work']);

    const ungranted = mintSessionFacadeToken('sess-worker', 'view', undefined, []);
    expect('profilesAllowed' in ungranted).toBe(false);
    expect('profilesAllowed' in rawTokens().find((r) => r.token === ungranted.token)!).toBe(false);
  });

  it('records a yoloAllowed grant only when true, and preserves it across rewrites', () => {
    // omitempty wire shape (TWIN: authtoken.Record.YoloAllowed) — present as
    // literal true, ABSENT when false.
    const yes = mintSessionFacadeToken('sess-yolo', 'operator', undefined, undefined, true);
    expect(yes.yoloAllowed).toBe(true);
    expect(rawTokens().find((r) => r.token === yes.token)!.yoloAllowed).toBe(true);

    getOrCreateRemoteToken('view', 'Dashboard');
    expect(rawTokens().find((r) => r.token === yes.token)!.yoloAllowed).toBe(true);

    const no = mintSessionFacadeToken('sess-safe', 'operator', undefined, undefined, false);
    expect('yoloAllowed' in no).toBe(false);
    expect('yoloAllowed' in rawTokens().find((r) => r.token === no.token)!).toBe(false);
  });

  it('re-minting for the same session replaces the old record (no stale scope/plugins)', () => {
    const first = mintSessionFacadeToken('sess-1', 'operator', ['a']);
    const second = mintSessionFacadeToken('sess-1', 'view');

    const raw = rawTokens();
    expect(raw).toHaveLength(1);
    expect(raw[0]).toMatchObject({ token: second.token, scope: 'view' });
    expect(raw[0].plugins).toBeUndefined();
    expect(raw.some((r) => r.token === first.token)).toBe(false);
  });

  it('is hidden from the Remote Control pairing list but preserved on rewrites', () => {
    const pairing = getOrCreateRemoteToken('triage', 'Phone pairing');
    const session = mintSessionFacadeToken('sess-1', 'view', ['p.x']);

    expect(listRemoteTokens()).toEqual([pairing]);
    // A pairing-list write cycle (mint another) must not strip the session
    // record or its plugins field.
    getOrCreateRemoteToken('view', 'Dashboard');
    const kept = rawTokens().find((r) => r.token === session.token);
    expect(kept).toMatchObject({ label: 'session:sess-1', plugins: ['p.x'] });
  });

  it('revokes by session id and sweeps against a live list', () => {
    mintSessionFacadeToken('sess-1', 'view');
    mintSessionFacadeToken('sess-2', 'triage');
    const pairing = getOrCreateRemoteToken('operator', 'Laptop');

    revokeSessionFacadeTokens('sess-1');
    expect(rawTokens().some((r) => r.label === 'session:sess-1')).toBe(false);

    // Sweep with only sess-3 live: sess-2's token goes, the pairing stays.
    mintSessionFacadeToken('sess-3', 'view');
    expect(sweepSessionFacadeTokens(['sess-3'])).toBe(1);
    const labels = rawTokens().map((r) => r.label);
    expect(labels).toContain('session:sess-3');
    expect(labels).not.toContain('session:sess-2');
    expect(rawTokens().some((r) => r.token === pairing.token)).toBe(true);

    // Nothing stale → no write, count 0.
    expect(sweepSessionFacadeTokens(['sess-3'])).toBe(0);
  });
});
