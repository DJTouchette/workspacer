/**
 * What "Default" means for a Claude session's reasoning effort.
 *
 * Claude Code takes `--effort <level>` at launch but reports the effective level
 * *nowhere*: the `system/init` frame carries `model` and `permissionMode` and no
 * effort field at all (verified on the wire — identical output with and without
 * `--effort max`), no hook payload carries it, and there is no `/effort` command.
 * So a session spawned without the flag can only be described by resolving the
 * same inputs the CLI resolves, which is what this does.
 *
 * The CLI's own resolver, read out of the 2.1.219 binary:
 *
 *     let t = <--effort flag>;
 *     if (t !== undefined) return t;
 *     if (settings.ultracode === true) return "xhigh";
 *     return settings.effortLevel;
 *
 * `effortLevel` is a settings key typed `enum(["low","medium","high","xhigh"])`
 * with `.catch(undefined)` — note `max` is accepted by the *flag* but not
 * persistable as a setting, and anything outside the enum is dropped rather than
 * honored. Both rules are mirrored here, so we can't name a level the CLI would
 * have ignored.
 *
 * Settings sources are read in Claude's documented precedence (project-local >
 * project > user). Enterprise `managed-settings.json` is deliberately NOT read:
 * its directory is constructed at runtime in the binary rather than appearing as
 * a literal, and reporting a level from the wrong source would be worse than
 * reporting none — a machine under an MDM policy that pins `effortLevel` falls
 * back to the old unresolved "Default".
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

/** Levels `effortLevel` can persist as, per the CLI's settings schema. */
const PERSISTABLE_EFFORT_LEVELS = ['low', 'medium', 'high', 'xhigh'] as const;

interface ClaudeSettingsSlice {
  effortLevel?: unknown;
  ultracode?: unknown;
}

/** Parse a settings file, tolerating absence and malformed JSON alike — the CLI
 *  ignores an unreadable settings file rather than failing, and so must we. */
function readSettings(file: string): ClaudeSettingsSlice | null {
  try {
    const parsed = JSON.parse(fs.readFileSync(file, 'utf8')) as unknown;
    return parsed && typeof parsed === 'object' ? (parsed as ClaudeSettingsSlice) : null;
  } catch {
    return null;
  }
}

/** The level this settings object pins, or null when it pins nothing valid. */
function levelFrom(settings: ClaudeSettingsSlice | null): string | null {
  if (!settings) return null;
  // `ultracode: true` overrides the persisted level (CLI order, above).
  if (settings.ultracode === true) return 'xhigh';
  const level = settings.effortLevel;
  return typeof level === 'string' &&
    (PERSISTABLE_EFFORT_LEVELS as readonly string[]).includes(level)
    ? level
    : null;
}

/**
 * The effort level a Claude session launched *without* `--effort` will run at,
 * or undefined when nothing pins one (then it's the model's own default, which
 * the CLI doesn't name — so neither do we).
 *
 * `configDir` is a profile's `CLAUDE_CONFIG_DIR` when one is pinned; the user
 * scope moves with it exactly as it does for the CLI.
 */
export function resolveClaudeDefaultEffort(cwd?: string, configDir?: string): string | undefined {
  const home = configDir?.replace(/^~/, os.homedir()) || path.join(os.homedir(), '.claude');
  const sources = [
    ...(cwd
      ? [
          path.join(cwd, '.claude', 'settings.local.json'),
          path.join(cwd, '.claude', 'settings.json'),
        ]
      : []),
    path.join(home, 'settings.json'),
  ];
  for (const file of sources) {
    const level = levelFrom(readSettings(file));
    if (level) return level;
  }
  return undefined;
}
