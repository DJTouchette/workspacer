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
  /** Is this the default profile? */
  isDefault: boolean;
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
    return this.profiles.find(p => p.id === id);
  }

  getDefaultProfile(): ClaudeProfile {
    return this.profiles.find(p => p.isDefault) ?? this.profiles[0];
  }

  addProfile(name: string, configDir: string, extraArgs: string[]): ClaudeProfile {
    const profile: ClaudeProfile = {
      id: crypto.randomUUID(),
      name,
      configDir: configDir.trim(),
      extraArgs,
      isDefault: this.profiles.length === 0,
    };
    this.profiles.push(profile);
    this.save();
    return profile;
  }

  updateProfile(id: string, updates: Partial<Omit<ClaudeProfile, 'id'>>): ClaudeProfile | null {
    const profile = this.profiles.find(p => p.id === id);
    if (!profile) return null;
    if (updates.name !== undefined) profile.name = updates.name;
    if (updates.configDir !== undefined) profile.configDir = updates.configDir.trim();
    if (updates.extraArgs !== undefined) profile.extraArgs = updates.extraArgs;
    if (updates.isDefault) {
      // Unset other defaults
      for (const p of this.profiles) p.isDefault = p.id === id;
    }
    this.save();
    return { ...profile };
  }

  removeProfile(id: string): void {
    if (id === 'default') return; // Can't remove default
    this.profiles = this.profiles.filter(p => p.id !== id);
    // Ensure there's still a default
    if (!this.profiles.some(p => p.isDefault) && this.profiles.length > 0) {
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
