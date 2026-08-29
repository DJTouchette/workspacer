import { describe, it, expect } from 'vitest';
import { profileFormFields, configRootNote, profilesForProvider } from './profileFields';
import { PROFILE_CAPS, PROFILE_PROVIDERS } from '../../../main/shared/agentProfiles';

describe('profileFormFields', () => {
  it('never leaves a field present and quietly inert', () => {
    for (const provider of PROFILE_PROVIDERS) {
      for (const [key, state] of Object.entries(profileFormFields(provider))) {
        // Shown-and-enabled, shown-and-explained, or absent. Nothing else.
        if (state.shown && state.disabled) {
          expect(state.why, `${provider}.${key} is disabled with no reason`).toBeTruthy();
          expect(state.why!.length).toBeGreaterThan(10);
        } else {
          expect(state.why, `${provider}.${key} carries a reason it does not need`).toBeUndefined();
        }
      }
    }
  });

  it('agrees with the capability table, field by field', () => {
    for (const provider of PROFILE_PROVIDERS) {
      const f = profileFormFields(provider);
      const caps = PROFILE_CAPS[provider];
      // Concept-absent fields are absent; capability-blocked ones are disabled.
      expect(f.preset.shown).toBe(caps.preset);
      expect(f.tokenEnvVar.shown).toBe(caps.tokenEnv);
      expect(f.weight.disabled).toBe(!caps.failoverWeight);
      expect(f.mcpItemIds.disabled).toBe(!caps.mcpItemIds);
      // The root is the profile primitive — every harness that has profiles
      // has one, so it is never off.
      expect(f.configDir).toEqual({ shown: true, disabled: false });
    }
  });

  it('offers Claude every field it has always had', () => {
    const f = profileFormFields('claude');
    expect(f.weight.disabled).toBe(false);
    expect(f.mcpItemIds.disabled).toBe(false);
    expect(f.preset.shown).toBe(false);
    expect(f.tokenEnvVar.shown).toBe(false);
  });

  it('offers Codex the preset and the failover weight, but not the MCP loadout', () => {
    const f = profileFormFields('codex');
    expect(f.preset.shown).toBe(true);
    expect(f.weight.disabled).toBe(false);
    expect(f.mcpItemIds.disabled).toBe(true);
    expect(f.mcpItemIds.why).toMatch(/mcp-config/i);
  });

  it('offers Copilot the token reference and NO failover weight', () => {
    const f = profileFormFields('copilot');
    expect(f.tokenEnvVar.shown).toBe(true);
    expect(f.weight.disabled).toBe(true);
    // The reason is the real one: no usage-window signal to rotate off.
    expect(f.weight.why).toMatch(/usage-window|usage window/i);
    expect(f.preset.shown).toBe(false);
  });
});

describe('configRootNote', () => {
  it('names the env var it sets, for every harness', () => {
    for (const provider of PROFILE_PROVIDERS) {
      expect(configRootNote(provider)).toContain(PROFILE_CAPS[provider].configRootEnv);
    }
  });

  it('says the root IS the account where it is, and is NOT where it is not', () => {
    expect(configRootNote('claude')).toMatch(/own login/i);
    expect(configRootNote('codex')).toMatch(/auth\.json/i);
    expect(configRootNote('copilot')).toMatch(/login is NOT here/i);
  });
});

describe('profilesForProvider', () => {
  const profiles = [
    { id: 'a', name: 'Personal' }, // absent provider = Claude, by history
    { id: 'b', name: 'Claude work', provider: 'claude' as const },
    { id: 'c', name: 'Codex work', provider: 'codex' as const },
    { id: 'd', name: 'Copilot', provider: 'copilot' as const },
  ];

  it('treats a profile with no provider as a Claude one', () => {
    expect(profilesForProvider(profiles, 'claude').map((p) => p.id)).toEqual(['a', 'b']);
  });

  it('never offers another harness’s config root', () => {
    expect(profilesForProvider(profiles, 'codex').map((p) => p.id)).toEqual(['c']);
    expect(profilesForProvider(profiles, 'copilot').map((p) => p.id)).toEqual(['d']);
  });

  it('returns nothing for a harness with no config root, so no picker renders', () => {
    expect(profilesForProvider(profiles, 'opencode')).toEqual([]);
    expect(profilesForProvider(profiles, 'pi')).toEqual([]);
  });
});
