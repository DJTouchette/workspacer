/**
 * Provider guards on the shared Triage-Inbox resolve path (AttentionContext
 * approve/answer → resolveApproval/resolveAnswer).
 *
 * The keystroke fallbacks in this module encode Claude's TUI (the 3-row
 * permission menu, the numeric question picker). A managed provider's session
 * (codex/opencode/pi) either has no PTY at all or a foreign TUI, so:
 *
 *   - resolveApproval must NEVER fall back to claudeWrite for a non-claude
 *     provider when /approve fails (typing Claude's menu rows into a codex TUI
 *     is garbage input that can select the wrong row);
 *   - resolveAnswer must go structurally via claudeAnswer for non-claude
 *     providers — their questions are the daemon's parked AskUserQuestion MCP
 *     call, which only POST /answer resolves.
 *
 * The claude path (provider undefined or 'claude') keeps its keystroke
 * behaviour byte-for-byte; these tests pin both sides.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { resolveApproval, resolveAnswer } from './resolveAttention';

const api = window.electronAPI as unknown as Record<string, ReturnType<typeof vi.fn>>;

/** Let the .catch() handler of a rejected /approve run. */
const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  api.claudeApprove = vi.fn().mockResolvedValue(undefined);
  api.claudeAnswer = vi.fn().mockResolvedValue(undefined);
  api.claudeWrite = vi.fn();
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('resolveApproval — provider guard on the keystroke fallback', () => {
  it.each(['codex', 'opencode', 'pi'])(
    'a failed /approve on a %s session never writes claude menu keystrokes',
    async (provider) => {
      api.claudeApprove = vi.fn().mockRejectedValue(new Error('409'));
      resolveApproval('s1', 'yes', false, provider);
      resolveApproval('s1', 'no', false, provider);
      resolveApproval('s1', 'always', false, provider);
      await flush();
      expect(api.claudeApprove).toHaveBeenCalledTimes(3);
      expect(api.claudeWrite).not.toHaveBeenCalled();
    },
  );

  it.each([undefined, 'claude'])(
    'the claude path (provider=%s) still drives the 3-row menu on /approve failure',
    async (provider) => {
      api.claudeApprove = vi.fn().mockRejectedValue(new Error('409'));

      resolveApproval('s1', 'yes', false, provider);
      await flush();
      expect(api.claudeWrite).toHaveBeenLastCalledWith('s1', '\r');

      resolveApproval('s1', 'no', false, provider);
      await flush();
      expect(api.claudeWrite).toHaveBeenLastCalledWith('s1', '\x1b[B\x1b[B\r');

      resolveApproval('s1', 'always', false, provider);
      await flush();
      expect(api.claudeWrite).toHaveBeenLastCalledWith('s1', '\x1b[B\r');

      expect(api.claudeWrite).toHaveBeenCalledTimes(3);
    },
  );

  it('a successful /approve never touches the PTY (any provider)', async () => {
    resolveApproval('s1', 'yes', false, 'claude');
    resolveApproval('s1', 'yes', false, 'codex');
    await flush();
    expect(api.claudeWrite).not.toHaveBeenCalled();
  });

  it('an active question picker suppresses the claude keystroke fallback', async () => {
    api.claudeApprove = vi.fn().mockRejectedValue(new Error('409'));
    resolveApproval('s1', 'yes', /* hasPendingQuestion */ true, 'claude');
    await flush();
    expect(api.claudeWrite).not.toHaveBeenCalled();
  });
});

describe('resolveAnswer — structural-only for managed providers', () => {
  it.each(['codex', 'opencode', 'pi'])(
    'provider=%s answers via claudeAnswer exactly once and never claudeWrite',
    async (provider) => {
      resolveAnswer('s1', { option: 2 }, provider);
      expect(api.claudeAnswer).toHaveBeenCalledTimes(1);
      expect(api.claudeAnswer).toHaveBeenCalledWith('s1', { option: 2 });
      await flush();
      expect(api.claudeWrite).not.toHaveBeenCalled();
    },
  );

  it('a failed /answer on a managed provider does NOT fall back to keystrokes', async () => {
    api.claudeAnswer = vi.fn().mockRejectedValue(new Error('409'));
    resolveAnswer('s1', { option: 1 }, 'codex');
    await flush();
    expect(api.claudeWrite).not.toHaveBeenCalled();
  });

  it.each([undefined, 'claude'])(
    'the claude path (provider=%s) writes option/text/answers as keystrokes, not /answer',
    (provider) => {
      resolveAnswer('s1', { option: 3 }, provider);
      expect(api.claudeWrite).toHaveBeenLastCalledWith('s1', '3\r');

      resolveAnswer('s1', { text: 'use sqlite' }, provider);
      expect(api.claudeWrite).toHaveBeenLastCalledWith('s1', 'use sqlite\r');

      resolveAnswer('s1', { answers: ['a', 'b'] }, provider);
      expect(api.claudeWrite).toHaveBeenCalledWith('s1', 'a\r');
      expect(api.claudeWrite).toHaveBeenCalledWith('s1', 'b\r');

      expect(api.claudeAnswer).not.toHaveBeenCalled();
    },
  );
});
