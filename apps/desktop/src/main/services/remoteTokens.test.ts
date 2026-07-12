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

  it('revokes a token from the shared hub token file', () => {
    const keep = getOrCreateRemoteToken('view', 'Dashboard');
    const remove = getOrCreateRemoteToken('triage', 'Phone pairing');

    expect(revokeRemoteToken(remove.token)).toEqual(remove);
    expect(listRemoteTokens()).toEqual([keep]);
    expect(JSON.parse(fs.readFileSync(tokensFile(), 'utf-8'))).toEqual([keep]);
    expect(() => revokeRemoteToken(remove.token)).toThrow(/token not found/);
  });
});
