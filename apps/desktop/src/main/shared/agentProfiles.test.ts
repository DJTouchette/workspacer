/**
 * The per-harness capability table — the one place that decides what a profile
 * MEANS for each CLI, read by both `main` (the spawn) and the renderer (the
 * form that sets it). A capability that says one thing here and another at the
 * spawn is the whole bug class this module exists to close, so every claim in
 * PROFILE_CAPS is pinned against the helper that acts on it.
 */
import { describe, it, expect } from 'vitest';
import {
  PROFILE_CAPS,
  PROFILE_PROVIDERS,
  clampProfileWeight,
  profileAppliesTo,
  profileConfigEnv,
  profileConfigRoot,
  profileProviderOf,
  profileSpawnArgs,
  providerTakesProfiles,
  sanitizeProfilePreset,
} from './agentProfiles';

const HOME = '/home/u';

describe('profileProviderOf — absent means claude', () => {
  it('reads a profile written before the field existed as a Claude profile', () => {
    expect(profileProviderOf({ configDir: '~/.claude-work' })).toBe('claude');
    expect(profileProviderOf(undefined)).toBe('claude');
  });

  it('an unrecognised provider falls back to claude rather than a broken root', () => {
    expect(profileProviderOf({ provider: 'gemini' })).toBe('claude');
  });
});

describe('profileConfigEnv — the config root is per harness', () => {
  it.each([
    ['claude', 'CLAUDE_CONFIG_DIR'],
    ['codex', 'CODEX_HOME'],
    ['copilot', 'COPILOT_HOME'],
  ])('a %s profile sets %s', (provider, envVar) => {
    expect(profileConfigEnv({ provider, configDir: '~/.alt' }, HOME)).toEqual({
      [envVar]: '/home/u/.alt',
    });
  });

  it('a profile naming no root contributes NOTHING — that is the default login', () => {
    expect(profileConfigEnv({ provider: 'codex', configDir: '' }, HOME)).toEqual({});
    expect(profileConfigEnv({ provider: 'codex', configDir: '   ' }, HOME)).toEqual({});
    expect(profileConfigEnv(undefined, HOME)).toEqual({});
  });
});

describe('profileSpawnArgs — extraArgs plus the native preset flag', () => {
  it("a codex preset becomes `-p <name>`, after the profile's own extraArgs", () => {
    expect(
      profileSpawnArgs({ provider: 'codex', extraArgs: ['--model', 'o3'], preset: 'work' }),
    ).toEqual(['--model', 'o3', '-p', 'work']);
  });

  it('a preset on a harness with no preset flag is NOT invented onto the argv', () => {
    expect(profileSpawnArgs({ provider: 'claude', extraArgs: [], preset: 'work' })).toEqual([]);
    expect(profileSpawnArgs({ provider: 'copilot', extraArgs: [], preset: 'work' })).toEqual([]);
  });

  it('a hand-written -p in extraArgs loses to the field labelled "preset"', () => {
    // Both reach codex; codex takes the LAST -p, which is the visible field.
    expect(
      profileSpawnArgs({ provider: 'codex', extraArgs: ['-p', 'typed'], preset: 'picked' }),
    ).toEqual(['-p', 'typed', '-p', 'picked']);
  });
});

describe('sanitizeProfilePreset — a preset name becomes a FILE PATH and an argv token', () => {
  it('keeps an ordinary config-profile stem', () => {
    expect(sanitizeProfilePreset('work-2')).toBe('work-2');
    expect(sanitizeProfilePreset('  gpt5_high  ')).toBe('gpt5_high');
  });

  it('strips a path escape — the name resolves to $CODEX_HOME/<name>.config.toml', () => {
    // The separators go, and what is left still starts with the traversal's
    // dots — which the leading-character rule rejects outright, so this is not
    // a preset at all rather than a mangled one.
    expect(sanitizeProfilePreset('../../etc/passwd')).toBe('');
    expect(sanitizeProfilePreset('a/b')).toBe('ab');
  });

  it('refuses a name codex would read as the NEXT FLAG rather than -p’s value', () => {
    expect(sanitizeProfilePreset('--dangerously-bypass-approvals-and-sandbox')).toBe('');
    expect(sanitizeProfilePreset('-x')).toBe('');
    // A leading dot would name a hidden file / a relative segment.
    expect(sanitizeProfilePreset('.hidden')).toBe('');
  });

  it('an empty or all-stripped name is "no preset", not a broken one', () => {
    expect(sanitizeProfilePreset(undefined)).toBe('');
    expect(sanitizeProfilePreset('   ')).toBe('');
    expect(sanitizeProfilePreset('!!!')).toBe('');
  });

  it('a stripped-to-nothing preset puts NO -p on the argv', () => {
    expect(profileSpawnArgs({ provider: 'codex', extraArgs: [], preset: '--yolo' })).toEqual([]);
  });
});

describe('clampProfileWeight — only harnesses with an exhaustion signal', () => {
  it('claude and codex keep a positive weight (both report usage windows)', () => {
    expect(clampProfileWeight('claude', 5)).toBe(5);
    expect(clampProfileWeight('codex', 5)).toBe(5);
  });

  it('copilot is forced to 0 — nothing could ever tell the rotation it is spent', () => {
    expect(clampProfileWeight('copilot', 5)).toBe(0);
    expect(PROFILE_CAPS.copilot.failoverWeight).toBe(false);
    expect(PROFILE_CAPS.copilot.whyNoFailoverWeight).toBeTruthy();
  });

  it('a non-number or non-positive weight is 0 everywhere', () => {
    expect(clampProfileWeight('claude', undefined)).toBe(0);
    expect(clampProfileWeight('claude', -3)).toBe(0);
    expect(clampProfileWeight('claude', '9')).toBe(0);
  });
});

describe('profileAppliesTo — a profile belongs to exactly one harness', () => {
  it('matches its own harness and nothing else', () => {
    expect(profileAppliesTo({ provider: 'codex' }, 'codex')).toBe(true);
    expect(profileAppliesTo({ provider: 'codex' }, 'claude')).toBe(false);
    expect(profileAppliesTo({ configDir: '~/.claude-work' }, 'claude')).toBe(true);
    expect(profileAppliesTo({ configDir: '~/.claude-work' }, 'codex')).toBe(false);
  });

  it('no profile applies to a harness that takes none', () => {
    expect(profileAppliesTo({ provider: 'claude' }, 'opencode')).toBe(false);
    expect(providerTakesProfiles('opencode')).toBe(false);
    expect(providerTakesProfiles('pi')).toBe(false);
  });
});

describe('profileConfigRoot — where the identity is read from', () => {
  it('uses the profile’s own root when it names one', () => {
    expect(profileConfigRoot({ provider: 'codex', configDir: '~/.codex-work' }, HOME)).toBe(
      '/home/u/.codex-work',
    );
  });

  it("falls back to the harness's default root, not to Claude's", () => {
    expect(profileConfigRoot({ provider: 'codex', configDir: '' }, HOME)).toBe('/home/u/.codex');
    expect(profileConfigRoot({ provider: 'copilot' }, HOME)).toBe('/home/u/.copilot');
    expect(profileConfigRoot(undefined, HOME)).toBe('/home/u/.claude');
  });

  it("honours the app's own root override — a rootless profile means THAT account", () => {
    expect(profileConfigRoot({ provider: 'codex' }, HOME, { CODEX_HOME: '~/alt-codex' })).toBe(
      '/home/u/alt-codex',
    );
  });
});

describe('every provider in the table is complete', () => {
  it.each(PROFILE_PROVIDERS)('%s declares a root env, a default root and a label', (p) => {
    const caps = PROFILE_CAPS[p];
    expect(caps.configRootEnv).toMatch(/^[A-Z_]+$/);
    expect(caps.defaultConfigRoot).toMatch(/^\./);
    expect(caps.label).toBeTruthy();
    expect(caps.configRootHint).toBeTruthy();
  });

  it('a capability that is OFF always says why, and one that is ON never invents a reason', () => {
    for (const p of PROFILE_PROVIDERS) {
      const caps = PROFILE_CAPS[p];
      expect(!!caps.whyNoFailoverWeight, `${p} failoverWeight`).toBe(!caps.failoverWeight);
      expect(!!caps.whyNoMcpItemIds, `${p} mcpItemIds`).toBe(!caps.mcpItemIds);
      // A preset claim without the flag that applies it would put nothing on argv.
      expect(!!caps.presetFlag, `${p} presetFlag`).toBe(caps.preset);
      expect(!!caps.presetHint, `${p} presetHint`).toBe(caps.preset);
    }
  });
});
