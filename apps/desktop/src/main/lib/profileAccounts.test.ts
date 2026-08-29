/**
 * Attribution read off disk, not guessed from a profile's name.
 *
 * The codex reader is the one that matters here: `$CODEX_HOME/auth.json` is a
 * CREDENTIAL file, so what this module lifts out of it (`tokens.account_id`,
 * `auth_mode` — and nothing else) is part of the contract, not an accident of
 * the parse. The tokens themselves must never reach an IPC reply.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { codexAccountFromAuthFile } from './profileAccounts';

let root = '';
beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-codexhome-'));
});
afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

function writeAuth(body: unknown): void {
  fs.writeFileSync(path.join(root, 'auth.json'), JSON.stringify(body));
}

describe('codexAccountFromAuthFile', () => {
  it('reports the STABLE account id and auth mode of a ChatGPT login', () => {
    writeAuth({
      auth_mode: 'chatgpt',
      OPENAI_API_KEY: null,
      tokens: {
        id_token: 'eyJ-secret',
        access_token: 'at-secret',
        refresh_token: 'rt-secret',
        account_id: 'acct_123',
      },
      last_refresh: '2026-08-01T00:00:00Z',
    });

    expect(codexAccountFromAuthFile(root)).toEqual({
      provider: 'codex',
      configRoot: root,
      signedIn: true,
      accountId: 'acct_123',
      authMode: 'chatgpt',
    });
  });

  it('never lifts a token out of the file — an identity is not a credential', () => {
    writeAuth({ auth_mode: 'chatgpt', tokens: { access_token: 'at-secret', account_id: 'a' } });
    const serialized = JSON.stringify(codexAccountFromAuthFile(root));
    expect(serialized).not.toContain('at-secret');
  });

  it('an API-key login counts as signed in even though it writes no account id', () => {
    writeAuth({ auth_mode: 'apikey', OPENAI_API_KEY: 'sk-x', tokens: {} });
    const got = codexAccountFromAuthFile(root);
    expect(got.signedIn).toBe(true);
    expect(got.accountId).toBeUndefined();
    expect(got.authMode).toBe('apikey');
  });

  it('a root with no auth.json is NOT signed in — the state the user must act on', () => {
    expect(codexAccountFromAuthFile(root).signedIn).toBe(false);
  });

  it('an unreadable or malformed auth.json reads as no login, never as a throw', () => {
    fs.writeFileSync(path.join(root, 'auth.json'), '{ not json');
    expect(codexAccountFromAuthFile(root).signedIn).toBe(false);
    expect(codexAccountFromAuthFile(path.join(root, 'nope')).signedIn).toBe(false);
  });
});
