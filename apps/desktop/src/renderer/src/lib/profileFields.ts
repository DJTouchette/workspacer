/**
 * Which profile fields a harness can actually honour — the renderer half of
 * `main/shared/agentProfiles`.
 *
 * The rule this module exists to enforce: A FIELD THAT CANNOT WORK FOR THE
 * SELECTED HARNESS IS NEVER PRESENT AND QUIETLY INERT. It is either absent, or
 * rendered disabled WITH THE REASON. Which of the two is not a style choice:
 *
 *   - ABSENT when the harness has no such concept at all. `codex -p` is a Codex
 *     feature; a `COPILOT_GITHUB_TOKEN` reference is a Copilot one. Showing
 *     Claude a greyed-out "preset" field would invent a capability gap where
 *     there is only a different design.
 *   - DISABLED WITH A REASON when the harness HAS the concept, the field lives
 *     on every profile row, and a user could reasonably expect it to work —
 *     failover weight and the Library MCP loadout are both stored on
 *     `ClaudeProfile` and both are FORCED to a neutral value at write time
 *     (clampProfileWeight / normalizeProfile). Those are the two that would
 *     otherwise accept a value and drop it, which is exactly the bug.
 *
 * Nothing is silently dropped either way: the `whyNoTokenEnv` of the harnesses
 * that don't take one is surfaced as the config-root note (`configRootNote`),
 * because "this root already holds its own login" is the reason Copilot is the
 * only harness that needs a token at all.
 *
 * CONSUMERS: components/settings/ClaudeProfilesSection.tsx (the form) and
 * components/SpawnAgentDialog.tsx (the picker). Both read this rather than
 * hard-coding a harness name, so the form and the spawn cannot disagree.
 */
import {
  PROFILE_CAPS,
  profileProviderOf,
  providerTakesProfiles,
  type ProfileLike,
  type ProfileProvider,
} from '../../../main/shared/agentProfiles';

/** How one field of the profile form behaves for the selected harness. */
export interface ProfileFieldState {
  /** false = do not render it at all (the harness has no such concept). */
  shown: boolean;
  /** true = render it, greyed out, with `why` beside it. Never accepts input. */
  disabled: boolean;
  /** Present exactly when `disabled` — the reason, in the user's words. */
  why?: string;
}

const ON: ProfileFieldState = { shown: true, disabled: false };
const OFF: ProfileFieldState = { shown: false, disabled: false };
const inert = (why: string): ProfileFieldState => ({ shown: true, disabled: true, why });

export interface ProfileFormFields {
  /** Always offered: every harness with a profile has a config root. */
  configDir: ProfileFieldState;
  /** Always offered: extra argv is a property of the command line, not the
   *  harness. */
  extraArgs: ProfileFieldState;
  preset: ProfileFieldState;
  tokenEnvVar: ProfileFieldState;
  weight: ProfileFieldState;
  mcpItemIds: ProfileFieldState;
}

/**
 * The form for one harness. Driven entirely by PROFILE_CAPS, so adding a
 * harness there is what turns its fields on — this file names no harness.
 */
export function profileFormFields(provider: ProfileProvider): ProfileFormFields {
  const caps = PROFILE_CAPS[provider];
  return {
    configDir: ON,
    extraArgs: ON,
    // Concept-absent → absent. Only Codex has a same-account settings preset.
    preset: caps.preset ? ON : OFF,
    // Concept-absent → absent. A token reference only exists where the root is
    // NOT the account; `configRootNote` says so for the harnesses where it is.
    tokenEnvVar: caps.tokenEnv ? ON : OFF,
    // Stored on the row and forced to 0 on write → disabled with the reason.
    weight: caps.failoverWeight
      ? ON
      : inert(caps.whyNoFailoverWeight ?? 'This harness reports no usage window to rotate off'),
    // Stored on the row and forced empty on write → disabled with the reason.
    mcpItemIds: caps.mcpItemIds
      ? ON
      : inert(caps.whyNoMcpItemIds ?? 'This harness does not take Claude’s --mcp-config'),
  };
}

/**
 * The note under the config-root field: what setting this root actually buys.
 * For the harnesses whose root IS the login that is the whole story, and it is
 * `whyNoTokenEnv` — the same string that explains why they are offered no token
 * reference. For Copilot the root is not the account, so the note says what the
 * root does and does NOT switch, which is the one thing about it that surprises.
 */
export function configRootNote(provider: ProfileProvider): string {
  const caps = PROFILE_CAPS[provider];
  if (caps.whyNoTokenEnv) return `${caps.configRootEnv} — ${caps.whyNoTokenEnv}.`;
  if (caps.tokenEnv)
    return `${caps.configRootEnv} — its own config, state, sessions and MCP servers. The login is NOT here (it lives in the OS credential store), so a second identity needs the token below.`;
  return `${caps.configRootEnv} — this harness’s config root.`;
}

/**
 * The profiles a spawn of `provider` may actually use.
 *
 * A Claude profile on a Codex spawn would point CODEX_HOME at a Claude config
 * root, so the picker filters and `services/managedSpawn` re-checks (the spawn
 * is also reachable from the hub bus, where no picker ran). A harness with no
 * config root gets an EMPTY list, which is what makes the picker disappear
 * rather than offer chips that set nothing.
 */
export function profilesForProvider<T extends ProfileLike>(
  profiles: readonly T[],
  provider: string,
): T[] {
  if (!providerTakesProfiles(provider)) return [];
  return profiles.filter((p) => profileProviderOf(p) === provider);
}
