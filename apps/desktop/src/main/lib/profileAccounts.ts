/**
 * Who a profile actually IS, read from the harness's own credential file
 * instead of inferred from the profile's name.
 *
 * A profile is a config ROOT (agentProfiles: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`,
 * `COPILOT_HOME`), and every harness that has one keeps its login inside it. So
 * the attribution a profile row shows — signed in or not, and WHICH account —
 * can be a fact read off disk rather than a guess:
 *
 *   claude  → `<root>/.credentials.json` (or `.claude.json`'s `oauthAccount`,
 *             which also carries the account uuid). `claudeAccountSetup`
 *             already owns the `~/.claude.json`-vs-`<root>/.claude.json`
 *             quirk, so this module defers to it for the boolean.
 *   codex   → `<root>/auth.json`: `tokens.account_id` is the STABLE id (the
 *             same one codex sends upstream) and `auth_mode` says whether the
 *             account is a ChatGPT login or an API key.
 *   copilot → nothing readable. `copilot login` stores its token in the OS
 *             credential store, falling back to a plaintext file under the
 *             root only when there is no store; there is no file we can read
 *             for identity. So copilot reports UNKNOWN — UNLESS the profile
 *             references a token variable, in which case the answerable
 *             question changes: not "is there a login" but "is the variable
 *             this profile names actually visible to this app", which is the
 *             one failure the user cannot otherwise see (the spawn would
 *             silently fall back to the default login).
 *
 * `signedIn: undefined` is therefore load-bearing and distinct from `false`:
 * false is "this root has no login yet", undefined is "this harness does not
 * tell us". Callers that need a boolean (the failover candidate filter) treat
 * undefined as "keep the candidate", the same as they always treated a Claude
 * profile with no configDir.
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  profileConfigRoot,
  profileProviderOf,
  sanitizeEnvVarName,
  type ProfileLike,
} from '../shared/agentProfiles';
import { accountLoginStatus } from '../services/claudeAccountSetup';
import type { ProfileAccount } from '../shared/ipcTypes';

export type { ProfileAccount };

function readJson(file: string): Record<string, unknown> | undefined {
  try {
    const parsed = JSON.parse(fs.readFileSync(file, 'utf-8')) as unknown;
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : undefined;
  } catch {
    return undefined;
  }
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() !== '' ? value.trim() : undefined;
}

/**
 * Codex's `$CODEX_HOME/auth.json`. Exported for its own test — the file shape
 * is the contract, and it is the one place this module reads a credential file
 * directly rather than through an existing helper.
 *
 * Only `auth_mode` and `tokens.account_id` are read. The tokens themselves are
 * deliberately NOT returned: nothing upstream needs them, and a secret that is
 * never lifted off disk cannot leak through an IPC reply.
 */
export function codexAccountFromAuthFile(root: string): ProfileAccount {
  const base: ProfileAccount = { provider: 'codex', configRoot: root };
  const auth = readJson(path.join(root, 'auth.json'));
  if (!auth) return { ...base, signedIn: false };
  const tokens = (auth.tokens ?? {}) as Record<string, unknown>;
  const accountId = str(tokens.account_id);
  const authMode = str(auth.auth_mode);
  // An API-key login writes no account_id, so "has an id" is not the test for
  // being signed in — either credential counts.
  const signedIn = !!accountId || !!str(auth.OPENAI_API_KEY) || !!str(tokens.access_token);
  return { ...base, signedIn, ...(accountId && { accountId }), ...(authMode && { authMode }) };
}

/** Claude's, via the existing account helper (it owns the `.claude.json` quirk). */
function claudeAccount(root: string, configDir: string | undefined): ProfileAccount {
  const base: ProfileAccount = { provider: 'claude', configRoot: root };
  // A profile with no configDir runs the primary login, which the app itself is
  // running under — treated as signed in, exactly as the login-status IPC
  // always has.
  if (!configDir?.trim()) return { ...base, signedIn: true, ...readClaudeAccountId(root) };
  return { ...base, signedIn: accountLoginStatus(configDir), ...readClaudeAccountId(root) };
}

function readClaudeAccountId(root: string): { accountId?: string } {
  const claudeJson = readJson(
    path.resolve(root) === path.join(os.homedir(), '.claude')
      ? path.join(os.homedir(), '.claude.json')
      : path.join(root, '.claude.json'),
  );
  const oauth = (claudeJson?.oauthAccount ?? {}) as Record<string, unknown>;
  const accountId = str(oauth.accountUuid) ?? str(oauth.emailAddress);
  return accountId ? { accountId } : {};
}

/**
 * The identity behind a profile. Never throws: an unreadable root is reported
 * as "no login here", which is what the user needs to act on anyway.
 *
 * CONSUMERS: ipc.ts `claude-profiles:accounts` (the Settings attribution chip)
 * and `claude-profiles:loginStatus` (the signed-in badge + the failover
 * candidate filter, which is why the boolean collapse lives in one place).
 */
export function profileAccount(profile: ProfileLike | undefined): ProfileAccount {
  const provider = profileProviderOf(profile);
  const root = profileConfigRoot(profile, os.homedir(), process.env);
  if (provider === 'claude') return claudeAccount(root, profile?.configDir);
  if (provider === 'codex') return codexAccountFromAuthFile(root);
  // copilot: the OS credential store holds the token, so the root tells us
  // nothing. A referenced variable does: report whether it resolves.
  const tokenEnvVar = sanitizeEnvVarName(profile?.tokenEnvVar);
  if (!tokenEnvVar) return { provider, configRoot: root };
  return {
    provider,
    configRoot: root,
    tokenEnvVar,
    signedIn: !!process.env[tokenEnvVar]?.trim(),
  };
}

/**
 * The boolean the signed-in badge and the failover filter take. UNKNOWABLE
 * collapses to true, matching the long-standing rule for a Claude profile with
 * no configDir: a candidate we cannot rule out stays in, and the spawn shows
 * the harness's own sign-in prompt if it was wrong.
 */
export function profileSignedIn(profile: ProfileLike | undefined): boolean {
  return profileAccount(profile).signedIn !== false;
}
