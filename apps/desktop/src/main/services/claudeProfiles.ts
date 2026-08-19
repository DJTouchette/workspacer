/**
 * Claude profile management — each profile has its own config dir and CLI args.
 * Stored in ~/.config/workspacer/claude-profiles.json
 */
import * as fs from 'fs';
import * as path from 'path';
import * as crypto from 'crypto';
import { getConfigDir } from './configService';

export interface ClaudeProfile {
  id: string;
  name: string;
  /** Custom CLAUDE_CONFIG_DIR (empty = use default ~/.claude) */
  configDir: string;
  /** Extra CLI args, e.g. ["--dangerously-skip-permissions"] */
  extraArgs: string[];
  /** Library item ids (kind 'mcp') to load by default when spawning with this
   *  profile. The spawn dialog pre-fills from these and lets the user override. */
  mcpItemIds?: string[];
  /** Is this the default profile? */
  isDefault: boolean;
  /** Automatic-failover weight. 0 (the default) keeps the profile manual-only;
   *  any positive value opts it into the rotation: when a session's account
   *  hits its usage window, the pane restarts onto the highest-weight
   *  signed-in profile that isn't already exhausted (lib/profileFailover on
   *  the renderer side). TWIN: `Weight` in cmd/brain/profiles.go — the brain
   *  round-trips the file, so a field it doesn't model gets WIPED on its next
   *  write. */
  weight?: number;
}

/**
 * The ALLOWLIST of profile extraArgs that survive onto a remote (bus/web/MCP)
 * spawn's argv, mapped to whether the flag takes a value.
 *
 * A denylist was the wrong shape: it named `--dangerously-skip-permissions` and
 * a bypass `--permission-mode`, while `--allowedTools` (blanket tool
 * auto-approval) and `--settings` (an arbitrary settings file: permissions AND
 * hooks) walked straight through and handed the bypass back. These three are
 * what a profile legitimately pins for a remote spawn; anything else is dropped
 * rather than reasoned about, so a flag added to the CLI tomorrow is denied by
 * default.
 */
const REMOTE_SAFE_FLAGS: Record<string, boolean> = {
  '--model': true,
  '--effort': true,
  '--permission-mode': true, // non-bypass modes only — see below
};

/**
 * Keep only REMOTE_SAFE_FLAGS (both `--flag value` and `--flag=value` forms),
 * and drop `--permission-mode` when it names a bypass mode.
 *
 * TWIN: `scrubBypassArgs` in services/hub/cmd/brain/profiles.go — the brain is
 * the DEFAULT answerer for a bus spawn, so a rule that lives on only one side
 * is a rule that doesn't run. The two implementations are line-for-line
 * equivalents; change them together.
 */
export function scrubBypassArgs(args: string[] | undefined): string[] {
  const out: string[] = [];
  const list = args ?? [];
  for (let i = 0; i < list.length; i++) {
    const eq = list[i].indexOf('=');
    const inline = eq !== -1;
    const name = inline ? list[i].slice(0, eq) : list[i];
    let value = inline ? list[i].slice(eq + 1) : '';
    const allowed = name in REMOTE_SAFE_FLAGS;
    const hasSeparateValue = allowed && REMOTE_SAFE_FLAGS[name] && !inline;
    if (hasSeparateValue) {
      // A flag whose value is the next element, unless it's missing or is
      // itself a flag — in which case the profile is malformed and we drop it.
      if (i + 1 >= list.length || list[i + 1].startsWith('-')) continue;
      value = list[i + 1];
    }
    if (!allowed) {
      // Drop the flag AND the value riding beside it: a dropped "--settings"
      // that left its path behind would hand claude a stray positional, which
      // it reads as the prompt.
      if (!inline && i + 1 < list.length && !list[i + 1].startsWith('-')) i++;
      continue;
    }
    if (name === '--permission-mode' && (value === 'bypassPermissions' || value === 'yolo')) {
      if (hasSeparateValue) i++;
      continue;
    }
    out.push(list[i]);
    if (hasSeparateValue) {
      i++;
      out.push(value);
    }
  }
  return out;
}

/**
 * The copy of a profile a remote (bus/web/MCP) spawn is allowed to use:
 * extraArgs reduced to the allowlist above, and no CLAUDE_CONFIG_DIR. Without
 * it, clamping the request's own fields left profileId as an open door — the
 * caller points at (or mints, since claude.profiles.add is itself a bus
 * capability) a profile that carries the bypass for them.
 *
 * configDir is dropped rather than contained: it becomes CLAUDE_CONFIG_DIR, and
 * that directory supplies claude's settings.json — permissions.allow and hooks,
 * i.e. commands claude runs unprompted. A bus caller can write files anywhere
 * inside an agent cwd (fs.write) and then name that directory in a profile, so
 * there is no subtree we could allow that the same caller can't also fill in. A
 * remote spawn therefore runs against the host's default claude config dir.
 *
 * mcpItemIds is dropped for the same reason, and it is the sharper one. A
 * library item of kind `mcp` carries `command`, `args` and `env` verbatim into a
 * `--mcp-config`, and the spawn passes `--allowedTools mcp__<id>` alongside it,
 * so the server is PRE-APPROVED and no prompt gates it — a persisted id list is
 * a persisted argv[0]. It was explicitly forwarded PAST this scrub on both
 * providers, so `claude.profiles.add` over the bus, then the local user picking
 * that profile in the New Agent dialog (SpawnAgentDialog copies
 * `p.mcpItemIds` into the spawn), was exactly the "wait for the LOCAL user,
 * where nothing scrubs" escalation this function's own comment describes.
 */
export function scrubBypassProfile<
  T extends { extraArgs?: string[]; configDir?: string; mcpItemIds?: string[] },
>(profile: T | undefined): T | undefined {
  return profile
    ? { ...profile, extraArgs: scrubBypassArgs(profile.extraArgs), configDir: '', mcpItemIds: [] }
    : profile;
}

const profilesFile = path.join(getConfigDir(), 'claude-profiles.json');

/** The row both providers write when the file has none. */
export const DEFAULT_PROFILE = (): ClaudeProfile => ({
  id: 'default',
  name: 'Default',
  configDir: '',
  extraArgs: [],
  mcpItemIds: [],
  isDefault: true,
  weight: 0,
});

/**
 * Fill the list fields the Go twin's normalizeProfiles fills.
 *
 * The brain has always served `extraArgs: []` / `mcpItemIds: []` rather than a
 * missing key (its own comment: "the same method answering with two different
 * shapes depending on which provider ran"), while this side returned the file
 * verbatim — so a profile written by hand, or by an older build, came back with
 * no `mcpItemIds` key at all here and with `[]` there.
 */
export function normalizeProfile(p: ClaudeProfile): ClaudeProfile {
  return {
    ...p,
    configDir: p.configDir ?? '',
    extraArgs: p.extraArgs ?? [],
    mcpItemIds: p.mcpItemIds ?? [],
    isDefault: p.isDefault === true,
    // Emit the key always (0 = manual-only), mirroring the Go twin's
    // no-omitempty int — same method, same shape, whichever provider answers.
    weight: typeof p.weight === 'number' && p.weight > 0 ? p.weight : 0,
  };
}

class ClaudeProfileService {
  private profiles: ClaudeProfile[] = [];

  constructor() {
    this.load();
    // Ensure there's always a default profile. The Go twin (cmd/brain
    // profiles.go loadProfiles) now MATERIALIZES the same row rather than
    // prepending a synthetic one it never wrote — that divergence made
    // claude.profiles.list return two profiles there and one here for the same
    // file, made the brain list an id its own update refused, and made
    // claude.profiles.add mint isDefault:true on one provider and false on the
    // other for the same call. Pinned by contracts/claude-profiles-cases.json.
    if (this.profiles.length === 0) {
      this.profiles.push(DEFAULT_PROFILE());
      this.save();
    }
  }

  getProfiles(): ClaudeProfile[] {
    return [...this.profiles];
  }

  getProfile(id: string): ClaudeProfile | undefined {
    return this.profiles.find((p) => p.id === id);
  }

  getDefaultProfile(): ClaudeProfile {
    return this.profiles.find((p) => p.isDefault) ?? this.profiles[0];
  }

  addProfile(
    name: string,
    configDir: string,
    extraArgs: string[],
    mcpItemIds: string[] = [],
  ): ClaudeProfile {
    const profile: ClaudeProfile = normalizeProfile({
      id: crypto.randomUUID(),
      name,
      configDir: configDir.trim(),
      extraArgs,
      mcpItemIds,
      isDefault: this.profiles.length === 0,
    });
    this.profiles.push(profile);
    this.save();
    return profile;
  }

  updateProfile(id: string, updates: Partial<Omit<ClaudeProfile, 'id'>>): ClaudeProfile | null {
    const profile = this.profiles.find((p) => p.id === id);
    if (!profile) return null;
    if (updates.name !== undefined) profile.name = updates.name;
    if (updates.configDir !== undefined) profile.configDir = updates.configDir.trim();
    if (updates.extraArgs !== undefined) profile.extraArgs = updates.extraArgs;
    if (updates.mcpItemIds !== undefined) profile.mcpItemIds = updates.mcpItemIds;
    if (updates.weight !== undefined) {
      profile.weight = typeof updates.weight === 'number' && updates.weight > 0 ? updates.weight : 0;
    }
    if (updates.isDefault) {
      // Unset other defaults
      for (const p of this.profiles) p.isDefault = p.id === id;
    }
    this.save();
    return { ...profile };
  }

  removeProfile(id: string): void {
    if (id === 'default') return; // Can't remove default
    this.profiles = this.profiles.filter((p) => p.id !== id);
    // Ensure there's still a default
    if (!this.profiles.some((p) => p.isDefault) && this.profiles.length > 0) {
      this.profiles[0].isDefault = true;
    }
    this.save();
  }

  private load(): void {
    try {
      if (fs.existsSync(profilesFile)) {
        const data = JSON.parse(fs.readFileSync(profilesFile, 'utf-8'));
        this.profiles = (data.profiles ?? []).map(normalizeProfile);
      }
    } catch {}
  }

  private save(): void {
    const dir = getConfigDir();
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(profilesFile, JSON.stringify({ profiles: this.profiles }, null, 2));
  }
}

export const claudeProfiles = new ClaudeProfileService();
