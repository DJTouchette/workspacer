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
 */
export function scrubBypassProfile<T extends { extraArgs?: string[]; configDir?: string }>(
  profile: T | undefined,
): T | undefined {
  return profile
    ? { ...profile, extraArgs: scrubBypassArgs(profile.extraArgs), configDir: '' }
    : profile;
}

const profilesFile = path.join(getConfigDir(), 'claude-profiles.json');

class ClaudeProfileService {
  private profiles: ClaudeProfile[] = [];

  constructor() {
    this.load();
    // Ensure there's always a default profile
    if (this.profiles.length === 0) {
      this.profiles.push({
        id: 'default',
        name: 'Default',
        configDir: '',
        extraArgs: [],
        isDefault: true,
      });
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
    const profile: ClaudeProfile = {
      id: crypto.randomUUID(),
      name,
      configDir: configDir.trim(),
      extraArgs,
      mcpItemIds,
      isDefault: this.profiles.length === 0,
    };
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
        this.profiles = data.profiles ?? [];
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
