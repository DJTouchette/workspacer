import { describe, it, expect, beforeEach, vi } from 'vitest';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

// Mock fs and os so we don't touch the real filesystem
vi.mock('fs');
vi.mock('os');

const mockHomedir = '/mock/home';

const { installHooks, uninstallHooks, hooksInstalled } = await import('../../src/main/services/claudeHooksConfig');

describe('claudeHooksConfig', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(os.homedir).mockReturnValue(mockHomedir);
    vi.mocked(fs.existsSync).mockReturnValue(true);
    vi.mocked(fs.mkdirSync).mockReturnValue(undefined as any);
    vi.mocked(fs.writeFileSync).mockReturnValue(undefined);
    // Default: no existing settings
    vi.mocked(fs.readFileSync).mockImplementation(() => { throw new Error('ENOENT'); });
  });

  function getWrittenSettings(): any {
    const calls = vi.mocked(fs.writeFileSync).mock.calls;
    if (calls.length === 0) return null;
    return JSON.parse(calls[calls.length - 1][1] as string);
  }

  describe('installHooks', () => {
    it('should create hooks config from scratch when settings.json does not exist', () => {
      const changed = installHooks();
      expect(changed).toBe(true);

      const written = getWrittenSettings();
      expect(Object.keys(written.hooks)).toHaveLength(11);
      expect(written.hooks.SessionStart).toBeDefined();
      expect(written.hooks.PostToolUse).toBeDefined();
      expect(written.hooks.PermissionRequest).toBeDefined();

      // Each event should have the curl command with marker
      const cmd = written.hooks.SessionStart[0].hooks[0].command;
      expect(cmd).toContain('curl');
      expect(cmd).toContain('localhost:7890/hook');
      expect(cmd).toContain('# workspacer-managed');
    });

    it('should merge into existing settings without overwriting user config', () => {
      const existingSettings = {
        permissions: { allow: ['Read'] },
        hooks: {
          PreToolUse: [
            {
              matcher: { tool_name: 'Bash' },
              hooks: [{ type: 'command', command: 'echo "user hook"' }],
            },
          ],
        },
      };
      vi.mocked(fs.readFileSync).mockReturnValue(JSON.stringify(existingSettings));

      const changed = installHooks();
      expect(changed).toBe(true);

      const written = getWrittenSettings();

      // User's permission config should be preserved
      expect(written.permissions).toEqual({ allow: ['Read'] });

      // User's existing PreToolUse hook should still be there
      const preToolUse = written.hooks.PreToolUse;
      const userMatcher = preToolUse.find((m: any) => m.matcher?.tool_name === 'Bash');
      expect(userMatcher).toBeDefined();
      expect(userMatcher.hooks[0].command).toBe('echo "user hook"');

      // Workspacer hook should be added in a catch-all matcher
      const catchAll = preToolUse.find((m: any) => !m.matcher);
      expect(catchAll).toBeDefined();
      expect(catchAll.hooks.some((h: any) => h.command.includes('workspacer-managed'))).toBe(true);
    });

    it('should not duplicate hooks if already installed', () => {
      // Build a full settings with the exact current command in every event
      const hooks: any = {};
      const events = ['SessionStart', 'SessionEnd', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'PostToolUseFailure', 'Notification', 'Stop', 'SubagentStart', 'SubagentStop', 'PermissionRequest'];
      // Use the exact command format that installHooks produces
      const exactCmd = "curl -s -X POST http://localhost:7890/hook -H 'Content-Type: application/json' -d @- # workspacer-managed";
      for (const event of events) {
        hooks[event] = [{ hooks: [{ type: 'command', command: exactCmd }] }];
      }
      vi.mocked(fs.readFileSync).mockReturnValue(JSON.stringify({ hooks }));

      const changed = installHooks();
      expect(changed).toBe(false);
      expect(fs.writeFileSync).not.toHaveBeenCalled();
    });
  });

  describe('uninstallHooks', () => {
    it('should remove only workspacer-managed hooks', () => {
      const settings = {
        hooks: {
          PreToolUse: [
            {
              matcher: { tool_name: 'Bash' },
              hooks: [{ type: 'command', command: 'echo "user hook"' }],
            },
            {
              hooks: [{ type: 'command', command: 'curl ... # workspacer-managed' }],
            },
          ],
          SessionStart: [
            {
              hooks: [{ type: 'command', command: 'curl ... # workspacer-managed' }],
            },
          ],
        },
      };
      vi.mocked(fs.readFileSync).mockReturnValue(JSON.stringify(settings));

      const changed = uninstallHooks();
      expect(changed).toBe(true);

      const written = getWrittenSettings();

      // User's PreToolUse hook should survive
      expect(written.hooks.PreToolUse).toHaveLength(1);
      expect(written.hooks.PreToolUse[0].hooks[0].command).toBe('echo "user hook"');

      // SessionStart should be fully removed
      expect(written.hooks.SessionStart).toBeUndefined();
    });

    it('should return false when no hooks exist', () => {
      vi.mocked(fs.readFileSync).mockReturnValue('{}');

      const changed = uninstallHooks();
      expect(changed).toBe(false);
      expect(fs.writeFileSync).not.toHaveBeenCalled();
    });

    it('should clean up empty hooks object', () => {
      const settings = {
        hooks: {
          SessionStart: [
            {
              hooks: [{ type: 'command', command: 'curl ... # workspacer-managed' }],
            },
          ],
        },
      };
      vi.mocked(fs.readFileSync).mockReturnValue(JSON.stringify(settings));

      uninstallHooks();

      const written = getWrittenSettings();
      expect(written.hooks).toBeUndefined();
    });
  });

  describe('hooksInstalled', () => {
    it('should return true when hooks are installed', () => {
      const settings = {
        hooks: {
          SessionStart: [
            { hooks: [{ type: 'command', command: 'curl ... # workspacer-managed' }] },
          ],
        },
      };
      vi.mocked(fs.readFileSync).mockReturnValue(JSON.stringify(settings));
      expect(hooksInstalled()).toBe(true);
    });

    it('should return false when no hooks exist', () => {
      vi.mocked(fs.readFileSync).mockReturnValue('{}');
      expect(hooksInstalled()).toBe(false);
    });

    it('should return false when only user hooks exist', () => {
      const settings = {
        hooks: {
          SessionStart: [
            { hooks: [{ type: 'command', command: 'echo "not workspacer"' }] },
          ],
        },
      };
      vi.mocked(fs.readFileSync).mockReturnValue(JSON.stringify(settings));
      expect(hooksInstalled()).toBe(false);
    });
  });
});
