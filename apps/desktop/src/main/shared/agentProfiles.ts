/**
 * What a PROFILE is, per harness — the one table both `main` and the renderer
 * read.
 *
 * A profile was born Claude-only and looked Claude-shaped: a `configDir` that
 * becomes `CLAUDE_CONFIG_DIR`, some extra argv, a Library MCP selection, and a
 * failover weight. Only two of those four are Claude-specific BY NATURE:
 *
 *   - `mcpItemIds` rides Claude's `--mcp-config`. Managed providers register
 *     MCP servers their own way, so a selection made for them would be a list
 *     nothing reads.
 *   - `weight` needs a per-account "this window is exhausted" signal to rotate
 *     on. Claude has one (statusLine / `rate_limit_event`) and so does Codex
 *     (`providers/mod.rs` `rate_limits_from` maps its primary/secondary windows
 *     onto the same `five_hour_pct` / `seven_day_pct` fields `windowExhausted`
 *     reads). Copilot emits no `RateLimits` update at all, so a weight on a
 *     Copilot profile could never fire.
 *
 * The config ROOT is Claude-specific only by history: every harness here has an
 * env var that relocates it, verified from each CLI's own help output —
 * `CLAUDE_CONFIG_DIR`, `CODEX_HOME` (already read by `lib/agentSkills.ts`),
 * `COPILOT_HOME`.
 *
 * Codex additionally has something Claude lacks, and CONFLATING THE TWO WOULD
 * BE THE BUG: `codex -p <name>` layers `$CODEX_HOME/<name>.config.toml` over
 * the base user config. That is a settings PRESET on the SAME account — same
 * login, same `auth.json`, same usage pool. It is not an account switch. So it
 * is modelled as its own optional field beside the root, never as an alias for
 * it.
 *
 * Every capability here is a claim about what the harness can do, and each one
 * has exactly one consumer that must agree with it:
 *
 *   configRootEnv  → services/managedSpawn.ts (the spawn env) via profileConfigEnv
 *   preset         → services/managedSpawn.ts (the spawn argv) via profileSpawnArgs
 *   mcpItemIds     → services/claudeProfiles.ts normalizeProfile (forced empty)
 *   failoverWeight → services/claudeProfiles.ts clampProfileWeight (forced 0)
 *                    + renderer lib/profileFailover.ts (candidate filter)
 *
 * Living in `main/shared` rather than `main/lib` is the point: the renderer
 * decides which FIELDS TO RENDER from this same table (settings form, spawn
 * dialog), so a capability cannot say one thing to the spawn path and another
 * to the control that sets it.
 */

/** The harnesses that have a config root, and therefore can have profiles. */
export type ProfileProvider = 'claude' | 'codex' | 'copilot';

export interface ProfileProviderCaps {
  /** The env var that relocates this harness's config root — the profile
   *  primitive. Set on the spawn's env; see profileConfigEnv. */
  configRootEnv: string;
  /** Placeholder/example root, for the field that edits it. */
  configRootHint: string;
  /** Whether this harness reports a per-account usage window we could rotate
   *  off. False here means a `weight` is FORCED TO 0 on write and the field is
   *  not offered — not that it is stored and ignored. */
  failoverWeight: boolean;
  /** Why not, when it isn't. Shown to the user rather than hiding the reason. */
  whyNoFailoverWeight?: string;
  /** Whether Library MCP selections mean anything here (Claude's
   *  `--mcp-config`). False forces the list empty on write. */
  mcpItemIds: boolean;
  whyNoMcpItemIds?: string;
  /** Whether this harness has a native SAME-ACCOUNT settings preset, layered
   *  over the base config of whichever root is in force. */
  preset: boolean;
  /** The flag that applies it (`-p`), when it has one. */
  presetFlag?: string;
  /** How the preset differs from the root switch, in the user's words. */
  presetHint?: string;
}

export const PROFILE_PROVIDERS: readonly ProfileProvider[] = ['claude', 'codex', 'copilot'];

export const PROFILE_CAPS: Readonly<Record<ProfileProvider, ProfileProviderCaps>> = {
  claude: {
    configRootEnv: 'CLAUDE_CONFIG_DIR',
    configRootHint: '~/.claude-work (blank = default ~/.claude)',
    failoverWeight: true,
    mcpItemIds: true,
    preset: false,
  },
  codex: {
    configRootEnv: 'CODEX_HOME',
    configRootHint: '~/.codex-work (blank = default ~/.codex)',
    // Codex reports primary/secondary usage windows, which claudemon maps onto
    // the same five_hour_pct / seven_day_pct the failover trigger reads.
    failoverWeight: true,
    mcpItemIds: false,
    whyNoMcpItemIds:
      'Library MCP selections ride Claude’s --mcp-config; Codex registers servers through its own config',
    preset: true,
    presetFlag: '-p',
    presetHint:
      'Layers $CODEX_HOME/<name>.config.toml over the base config. Same account, same usage pool — a settings preset, not a login switch.',
  },
  copilot: {
    configRootEnv: 'COPILOT_HOME',
    configRootHint: '~/.copilot-work (blank = default ~/.copilot)',
    // The one genuine hole: providers/copilot.rs emits no RateLimits update —
    // it reports token/AIU counts, never a window percentage — so nothing could
    // ever tell the rotation that a Copilot account is spent.
    failoverWeight: false,
    whyNoFailoverWeight:
      'Copilot reports no usage-window percentage, so nothing could ever detect that this account is exhausted',
    mcpItemIds: false,
    whyNoMcpItemIds:
      'Library MCP selections ride Claude’s --mcp-config; Copilot takes MCP servers its own way',
    preset: false,
  },
};

/** The shape every helper here reads. Structural so both `ClaudeProfile` (main)
 *  and the renderer's narrower row types satisfy it without importing each
 *  other. */
export interface ProfileLike {
  provider?: string;
  configDir?: string;
  extraArgs?: string[];
  preset?: string;
  weight?: number;
}

export function isProfileProvider(value: unknown): value is ProfileProvider {
  return value === 'claude' || value === 'codex' || value === 'copilot';
}

/**
 * Whether an AGENT provider can carry a profile at all. OpenCode and Pi have no
 * config-root override we have verified, so they get no profile rather than a
 * picker that resolves to nothing.
 */
export function providerTakesProfiles(provider: string): provider is ProfileProvider {
  return isProfileProvider(provider);
}

/**
 * A profile's harness. ABSENT MEANS CLAUDE, and that is load-bearing rather
 * than a default: every profile written before this field existed is a Claude
 * profile, and they are in daily use. Nothing migrates them, and nothing may
 * need to.
 */
export function profileProviderOf(profile: ProfileLike | undefined): ProfileProvider {
  return isProfileProvider(profile?.provider) ? profile.provider : 'claude';
}

export function profileCapsOf(profile: ProfileLike | undefined): ProfileProviderCaps {
  return PROFILE_CAPS[profileProviderOf(profile)];
}

/**
 * Whether this profile may be used for a spawn of `provider`. A Claude profile
 * on a Codex spawn would set the wrong root entirely, so the pickers filter on
 * this and the spawn path re-checks it — the spawn is reachable from the bus,
 * where no picker ran.
 */
export function profileAppliesTo(profile: ProfileLike | undefined, provider: string): boolean {
  return !!profile && profileProviderOf(profile) === provider;
}

/** `~` expansion, the one form these fields are written in. */
export function expandProfileHome(dir: string, home: string): string {
  return dir.replace(/^~/, home);
}

/**
 * The env this profile contributes to a spawn: its harness's config-root
 * variable, or nothing when the profile names no root (the default login).
 *
 * CONSUMER: services/managedSpawn.ts merges this into the spawn payload's `env`
 * — and, for Claude PTY, services/claudeSpawn.ts does the same by hand.
 */
export function profileConfigEnv(
  profile: ProfileLike | undefined,
  home: string,
): Record<string, string> {
  const dir = profile?.configDir?.trim();
  if (!profile || !dir) return {};
  return { [profileCapsOf(profile).configRootEnv]: expandProfileHome(dir, home) };
}

/**
 * The argv this profile contributes: its own `extraArgs`, plus the native
 * preset flag when the harness has one and the profile set it.
 *
 * The preset is appended AFTER extraArgs so a hand-written `-p` in extraArgs
 * loses to the field that is actually labelled "preset" in the UI — one place
 * decides, and it is the visible one.
 */
export function profileSpawnArgs(profile: ProfileLike | undefined): string[] {
  if (!profile) return [];
  const caps = profileCapsOf(profile);
  const args = [...(profile.extraArgs ?? [])];
  const preset = profile.preset?.trim();
  if (caps.preset && caps.presetFlag && preset) args.push(caps.presetFlag, preset);
  return args;
}

/**
 * The weight this profile is allowed to hold. A harness with no exhaustion
 * signal is clamped to 0 AT WRITE TIME rather than filtered at read time, so
 * the stored file never contains a number that looks like an opt-in and isn't.
 */
export function clampProfileWeight(provider: ProfileProvider, weight: unknown): number {
  if (!PROFILE_CAPS[provider].failoverWeight) return 0;
  return typeof weight === 'number' && weight > 0 ? weight : 0;
}
