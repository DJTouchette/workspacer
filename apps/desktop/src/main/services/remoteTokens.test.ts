import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const h = vi.hoisted(() => ({ dir: '' }));
vi.mock('./configService', () => ({ getConfigDir: () => h.dir }));

import { getOrCreateRemoteToken, listRemoteTokens, revokeRemoteToken } from './remoteTokens';

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
