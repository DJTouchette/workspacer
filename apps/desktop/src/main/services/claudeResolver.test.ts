/**
 * Characterization tests for buildClaudeArgv in claudeResolver.ts.
 *
 * Strategy: mock 'fs' so that findClaudeOnPath never finds a shim on PATH,
 * letting getBaseArgv fall back to { argv: ['claude'] } on Linux (non-win32).
 * This makes the base argv predictable without touching production logic.
 */

import { describe, it, expect, vi, beforeAll } from 'vitest';

// Mock 'fs' before importing the module under test so the cached base argv
// resolves to the fallback ['claude'] on Linux.
vi.mock('fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('fs')>();
  return {
    ...actual,
    existsSync: vi.fn().mockReturnValue(false),
  };
});

// Dynamic import AFTER the mock is in place, so cached resolves on first call.
const { buildClaudeArgv } = await import('./claudeResolver');

describe('buildClaudeArgv', () => {
  // On Linux with fs.existsSync always false, findClaudeOnPath returns null
  // and getBaseArgv falls back to ['claude'].
  const BASE = 'claude';

  describe('base argv', () => {
    it('returns ["claude", ...] as the base on Linux with no shim found', () => {
      const argv = buildClaudeArgv();
      expect(argv[0]).toBe(BASE);
    });

    it('returns a fresh array each call (not the same reference)', () => {
      const a = buildClaudeArgv();
      const b = buildClaudeArgv();
      expect(a).not.toBe(b);
    });
  });

  describe('extraArgs', () => {
    it('appends extraArgs when provided', () => {
      const argv = buildClaudeArgv({ extraArgs: ['--foo', 'bar'] });
      expect(argv).toContain('--foo');
      expect(argv).toContain('bar');
    });

    it('does not append extraArgs when empty array', () => {
      const argv = buildClaudeArgv({ extraArgs: [] });
      expect(argv).toEqual([BASE]);
    });

    it('does not append extraArgs when undefined', () => {
      const argv = buildClaudeArgv({});
      expect(argv).toEqual([BASE]);
    });
  });

  describe('--model flag injection', () => {
    it('injects --model when model is set and profile does not pin one', () => {
      const argv = buildClaudeArgv({ model: 'claude-opus-4-8' });
      expect(argv).toContain('--model');
      expect(argv).toContain('claude-opus-4-8');
      const idx = argv.indexOf('--model');
      expect(argv[idx + 1]).toBe('claude-opus-4-8');
    });

    it('trims whitespace from the model string', () => {
      const argv = buildClaudeArgv({ model: '  claude-sonnet-4-5  ' });
      const idx = argv.indexOf('--model');
      expect(idx).not.toBe(-1);
      expect(argv[idx + 1]).toBe('claude-sonnet-4-5');
    });

    it('does NOT inject --model when model is an empty string', () => {
      const argv = buildClaudeArgv({ model: '' });
      expect(argv).not.toContain('--model');
    });

    it('does NOT inject --model when model is whitespace-only', () => {
      const argv = buildClaudeArgv({ model: '   ' });
      expect(argv).not.toContain('--model');
    });

    it('does NOT inject --model when model is undefined', () => {
      const argv = buildClaudeArgv({});
      expect(argv).not.toContain('--model');
    });

    it('does NOT inject --model when profile extraArgs already contain --model as a separate flag', () => {
      const argv = buildClaudeArgv({
        model: 'claude-opus-4-8',
        extraArgs: ['--model', 'claude-haiku-4-5'],
      });
      // The profile's value should be present; the opts.model should NOT add another --model
      const count = argv.filter(a => a === '--model').length;
      expect(count).toBe(1);
      // The profile's model value appears right after the single --model
      const idx = argv.indexOf('--model');
      expect(argv[idx + 1]).toBe('claude-haiku-4-5');
    });

    it('does NOT inject --model when profile extraArgs contain --model=<value> form', () => {
      const argv = buildClaudeArgv({
        model: 'claude-opus-4-8',
        extraArgs: ['--model=claude-haiku-4-5'],
      });
      expect(argv).not.toContain('--model');
      expect(argv).toContain('--model=claude-haiku-4-5');
    });
  });

  describe('--dangerously-skip-permissions flag', () => {
    it('injects --dangerously-skip-permissions when skipPermissions is true', () => {
      const argv = buildClaudeArgv({ skipPermissions: true });
      expect(argv).toContain('--dangerously-skip-permissions');
    });

    it('does NOT inject when skipPermissions is false', () => {
      const argv = buildClaudeArgv({ skipPermissions: false });
      expect(argv).not.toContain('--dangerously-skip-permissions');
    });

    it('does NOT inject when skipPermissions is undefined', () => {
      const argv = buildClaudeArgv({});
      expect(argv).not.toContain('--dangerously-skip-permissions');
    });

    it('does NOT inject --dangerously-skip-permissions when profile extraArgs already contain it', () => {
      const argv = buildClaudeArgv({
        skipPermissions: true,
        extraArgs: ['--dangerously-skip-permissions'],
      });
      const count = argv.filter(a => a === '--dangerously-skip-permissions').length;
      expect(count).toBe(1);
    });

    it('still injects when profile has a different flag but not the skip-permissions one', () => {
      const argv = buildClaudeArgv({
        skipPermissions: true,
        extraArgs: ['--foo'],
      });
      expect(argv).toContain('--dangerously-skip-permissions');
    });
  });

  describe('--resume vs --session-id (mutually exclusive)', () => {
    it('adds --resume <id> when resumeSessionId is provided', () => {
      const argv = buildClaudeArgv({ resumeSessionId: 'abc-123' });
      expect(argv).toContain('--resume');
      const idx = argv.indexOf('--resume');
      expect(argv[idx + 1]).toBe('abc-123');
    });

    it('does NOT add --session-id when resumeSessionId is provided (resume wins)', () => {
      const argv = buildClaudeArgv({
        resumeSessionId: 'abc-123',
        sessionId: 'new-session-uuid',
      });
      expect(argv).toContain('--resume');
      expect(argv).not.toContain('--session-id');
    });

    it('adds --session-id when sessionId is provided and no resumeSessionId', () => {
      const argv = buildClaudeArgv({ sessionId: 'new-session-uuid' });
      expect(argv).toContain('--session-id');
      const idx = argv.indexOf('--session-id');
      expect(argv[idx + 1]).toBe('new-session-uuid');
    });

    it('does NOT add --resume when resumeSessionId is absent', () => {
      const argv = buildClaudeArgv({ sessionId: 'some-uuid' });
      expect(argv).not.toContain('--resume');
    });

    it('adds neither --resume nor --session-id when both are absent', () => {
      const argv = buildClaudeArgv({});
      expect(argv).not.toContain('--resume');
      expect(argv).not.toContain('--session-id');
    });
  });

  describe('profile-already-pins-session-id guard', () => {
    it('does NOT inject --session-id when profile extraArgs already contain --session-id as separate flags', () => {
      const argv = buildClaudeArgv({
        sessionId: 'new-uuid',
        extraArgs: ['--session-id', 'profile-pinned-uuid'],
      });
      // Profile value should be there, but no second --session-id injection
      const count = argv.filter(a => a === '--session-id').length;
      expect(count).toBe(1);
      const idx = argv.indexOf('--session-id');
      expect(argv[idx + 1]).toBe('profile-pinned-uuid');
    });

    it('does NOT inject --session-id when profile extraArgs contain --session-id=<value> form', () => {
      const argv = buildClaudeArgv({
        sessionId: 'new-uuid',
        extraArgs: ['--session-id=profile-pinned-uuid'],
      });
      expect(argv).not.toContain('--session-id');
      expect(argv).toContain('--session-id=profile-pinned-uuid');
    });

    it('does NOT inject --session-id when resumeSessionId is set even if profile also has extraArgs', () => {
      const argv = buildClaudeArgv({
        resumeSessionId: 'resume-uuid',
        sessionId: 'new-uuid',
        extraArgs: ['--foo'],
      });
      expect(argv).toContain('--resume');
      expect(argv).not.toContain('--session-id');
    });
  });

  describe('combined flag scenarios', () => {
    it('can combine model + skipPermissions + sessionId', () => {
      const argv = buildClaudeArgv({
        model: 'claude-opus-4-8',
        skipPermissions: true,
        sessionId: 'test-uuid',
      });
      expect(argv).toContain('--model');
      expect(argv).toContain('claude-opus-4-8');
      expect(argv).toContain('--dangerously-skip-permissions');
      expect(argv).toContain('--session-id');
      expect(argv).toContain('test-uuid');
    });

    it('can combine model + skipPermissions + resume', () => {
      const argv = buildClaudeArgv({
        model: 'claude-sonnet-4-5',
        skipPermissions: true,
        resumeSessionId: 'resume-uuid',
      });
      expect(argv).toContain('--model');
      expect(argv).toContain('--dangerously-skip-permissions');
      expect(argv).toContain('--resume');
      expect(argv).not.toContain('--session-id');
    });

    it('profile pins model and skips permissions; no opts inject them again', () => {
      const argv = buildClaudeArgv({
        model: 'claude-opus-4-8',
        skipPermissions: true,
        extraArgs: ['--model', 'profile-model', '--dangerously-skip-permissions'],
      });
      const modelCount = argv.filter(a => a === '--model').length;
      const skipCount = argv.filter(a => a === '--dangerously-skip-permissions').length;
      expect(modelCount).toBe(1);
      expect(skipCount).toBe(1);
    });

    it('ordering: extraArgs come first, then model, then skipPermissions, then session flags', () => {
      const argv = buildClaudeArgv({
        extraArgs: ['--extra-flag'],
        model: 'my-model',
        skipPermissions: true,
        sessionId: 'my-session',
      });
      // argv[0] = 'claude', then extraArgs, then model, then skip, then session-id
      expect(argv[0]).toBe(BASE);
      const extraIdx = argv.indexOf('--extra-flag');
      const modelIdx = argv.indexOf('--model');
      const skipIdx = argv.indexOf('--dangerously-skip-permissions');
      const sessionIdx = argv.indexOf('--session-id');
      expect(extraIdx).toBeLessThan(modelIdx);
      expect(modelIdx).toBeLessThan(skipIdx);
      expect(skipIdx).toBeLessThan(sessionIdx);
    });
  });

  describe('mcpConfig + strictMcpConfig', () => {
    it('emits --mcp-config <path> when mcpConfig is set', () => {
      const argv = buildClaudeArgv({ mcpConfig: '/tmp/session-mcp/abc.json' });
      const idx = argv.indexOf('--mcp-config');
      expect(idx).toBeGreaterThan(-1);
      expect(argv[idx + 1]).toBe('/tmp/session-mcp/abc.json');
    });

    it('does NOT emit --mcp-config or --strict-mcp-config when mcpConfig is unset', () => {
      const argv = buildClaudeArgv({ strictMcpConfig: true });
      expect(argv).not.toContain('--mcp-config');
      expect(argv).not.toContain('--strict-mcp-config');
    });

    it('adds --strict-mcp-config only alongside an mcpConfig', () => {
      const argv = buildClaudeArgv({ mcpConfig: '/tmp/x.json', strictMcpConfig: true });
      expect(argv).toContain('--strict-mcp-config');
      const cfgIdx = argv.indexOf('--mcp-config');
      const strictIdx = argv.indexOf('--strict-mcp-config');
      expect(cfgIdx).toBeLessThan(strictIdx);
    });

    it('omits --strict-mcp-config when strictMcpConfig is false', () => {
      const argv = buildClaudeArgv({ mcpConfig: '/tmp/x.json' });
      expect(argv).toContain('--mcp-config');
      expect(argv).not.toContain('--strict-mcp-config');
    });
  });
});
