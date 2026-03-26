/**
 * Auto-configures Claude Code hooks in ~/.claude/settings.json so that
 * hook events are forwarded to the Workspacer hook server.
 *
 * On install: merges Workspacer hook entries into the user's existing config.
 * On uninstall: removes only the Workspacer-managed entries, leaving user hooks intact.
 */

import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

const HOOK_PORT = 7890;
// Double quotes work in both bash/sh and Windows cmd/powershell
const HOOK_COMMAND = `curl -s -X POST http://localhost:${HOOK_PORT}/hook -H "Content-Type: application/json" -d @-`;

// Identify our hooks by the unique URL pattern (works regardless of quoting/comment style)
const WORKSPACER_MARKER = `localhost:${HOOK_PORT}/hook`;
// Also detect old-style hooks that used a bash comment marker
const OLD_MARKER = '# workspacer-managed';

const HOOK_EVENTS = [
  'SessionStart',
  'SessionEnd',
  'UserPromptSubmit',
  'PreToolUse',
  'PostToolUse',
  'PostToolUseFailure',
  'Notification',
  'Stop',
  'SubagentStart',
  'SubagentStop',
  'PermissionRequest',
];

function getClaudeSettingsPath(): string {
  return path.join(os.homedir(), '.claude', 'settings.json');
}

function readSettings(): any {
  const settingsPath = getClaudeSettingsPath();
  try {
    const raw = fs.readFileSync(settingsPath, 'utf-8');
    return JSON.parse(raw);
  } catch {
    return {};
  }
}

function writeSettings(settings: any): void {
  const settingsPath = getClaudeSettingsPath();
  const dir = path.dirname(settingsPath);

  // Ensure ~/.claude/ exists
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }

  fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2) + '\n', 'utf-8');
}

function makeHookEntry() {
  return {
    type: 'command' as const,
    command: HOOK_COMMAND,
  };
}

/**
 * Check if a hook array already contains a Workspacer-managed hook
 * with the CURRENT command format (not a stale one).
 */
function hasWorkspacerHook(hooks: any[]): boolean {
  return hooks.some(
    (h: any) => typeof h.command === 'string' && h.command === HOOK_COMMAND,
  );
}

/**
 * Check if a hook array contains any Workspacer-managed hook (current or stale).
 */
function hasAnyWorkspacerHook(hooks: any[]): boolean {
  return hooks.some(
    (h: any) =>
      typeof h.command === 'string' &&
      (h.command.includes(WORKSPACER_MARKER) || h.command.includes(OLD_MARKER)),
  );
}

/**
 * Install Workspacer hooks into ~/.claude/settings.json.
 * Merges non-destructively — existing user hooks are preserved.
 * Returns true if changes were made.
 */
export function installHooks(): boolean {
  const settings = readSettings();

  if (!settings.hooks) {
    settings.hooks = {};
  }

  let changed = false;

  for (const event of HOOK_EVENTS) {
    if (!settings.hooks[event]) {
      // No hooks for this event at all — create the structure
      settings.hooks[event] = [
        {
          hooks: [makeHookEntry()],
        },
      ];
      changed = true;
    } else if (Array.isArray(settings.hooks[event])) {
      // Event exists — check if our hook is already in any matcher group
      const matchers: any[] = settings.hooks[event];
      const alreadyInstalled = matchers.some(
        (matcher: any) => Array.isArray(matcher.hooks) && hasWorkspacerHook(matcher.hooks),
      );

      if (!alreadyInstalled) {
        // Remove any stale workspacer hooks first
        for (const matcher of matchers) {
          if (Array.isArray(matcher.hooks) && hasAnyWorkspacerHook(matcher.hooks)) {
            matcher.hooks = matcher.hooks.filter(
              (h: any) =>
                !(typeof h.command === 'string' &&
                  (h.command.includes(WORKSPACER_MARKER) || h.command.includes(OLD_MARKER))),
            );
          }
        }

        // Find a matcher without a tool_name filter (catch-all) or create one
        let catchAll = matchers.find((m: any) => !m.matcher);
        if (!catchAll) {
          catchAll = { hooks: [] };
          matchers.push(catchAll);
        }
        if (!Array.isArray(catchAll.hooks)) {
          catchAll.hooks = [];
        }
        catchAll.hooks.push(makeHookEntry());
        changed = true;
      }
    }
  }

  if (changed) {
    writeSettings(settings);
    console.log('[ClaudeHooks] installed hook config into ~/.claude/settings.json');
  } else {
    console.log('[ClaudeHooks] hooks already configured');
  }

  return changed;
}

/**
 * Remove Workspacer-managed hooks from ~/.claude/settings.json.
 * Leaves any user-created hooks intact.
 * Returns true if changes were made.
 */
export function uninstallHooks(): boolean {
  const settings = readSettings();
  if (!settings.hooks) return false;

  let changed = false;

  for (const event of HOOK_EVENTS) {
    if (!Array.isArray(settings.hooks[event])) continue;

    for (const matcher of settings.hooks[event]) {
      if (!Array.isArray(matcher.hooks)) continue;

      const before = matcher.hooks.length;
      matcher.hooks = matcher.hooks.filter(
        (h: any) =>
          !(typeof h.command === 'string' &&
            (h.command.includes(WORKSPACER_MARKER) || h.command.includes(OLD_MARKER))),
      );

      if (matcher.hooks.length < before) {
        changed = true;
      }
    }

    // Clean up empty matchers
    settings.hooks[event] = settings.hooks[event].filter(
      (matcher: any) => Array.isArray(matcher.hooks) && matcher.hooks.length > 0,
    );

    // Clean up empty event arrays
    if (settings.hooks[event].length === 0) {
      delete settings.hooks[event];
    }
  }

  // Clean up empty hooks object
  if (Object.keys(settings.hooks).length === 0) {
    delete settings.hooks;
  }

  if (changed) {
    writeSettings(settings);
    console.log('[ClaudeHooks] removed hook config from ~/.claude/settings.json');
  }

  return changed;
}

/**
 * Check if hooks are currently installed.
 */
export function hooksInstalled(): boolean {
  const settings = readSettings();
  if (!settings.hooks) return false;

  // Check that at least SessionStart has our hook
  const sessionStart = settings.hooks.SessionStart;
  if (!Array.isArray(sessionStart)) return false;

  return sessionStart.some(
    (matcher: any) => Array.isArray(matcher.hooks) && hasAnyWorkspacerHook(matcher.hooks),
  );
}
