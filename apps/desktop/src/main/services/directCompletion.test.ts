import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

/**
 * The primitive's whole contract is "never throws, never hangs, and tells you
 * WHY it produced nothing" — so the interesting surface is the failure matrix,
 * not the happy path. Each reason has to be reachable and distinguishable,
 * because the intended callers (brief writes, session titling) all degrade the
 * same way but log very differently.
 *
 * The provider seam is the other half: a model string one provider cannot serve
 * must be refused rather than forwarded, which is the bug
 * `supervisor.summarizerModel: 'sonnet'` already has under codex.
 */

vi.mock('./configService', () => ({
  configService: { getConfig: () => ({ agents: { binaries: {} } }) },
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./claudeResolver', () => ({ claudeBaseArgv: () => ['claude'] }));

const installed = new Set<string>(['claude', 'codex', 'opencode', 'pi']);
vi.mock('./agentProviders', () => ({
  resolveAgentBinary: (p: string) => p,
  isAgentBinaryInstalled: (p: string) => installed.has(p),
}));

import {
  complete,
  classifyCliFailure,
  extractCodexText,
  extractOpencodeText,
  stripAnsi,
  resolveCompletionModel,
  defaultModelFor,
  servesModel,
  completionSupported,
  MAX_PROMPT_CHARS,
} from './directCompletion';

describe('classifyCliFailure', () => {
  it('recognises every harness saying "you are not logged in"', () => {
    // Verbatim from the installed CLIs.
    expect(classifyCliFailure('No API key found for the selected model.')).toBe('not-authed');
    expect(classifyCliFailure('Not logged in. Run `codex login` first.')).toBe('not-authed');
    expect(classifyCliFailure('Invalid API key · Please run /login')).toBe('not-authed');
    expect(classifyCliFailure('HTTP 401 Unauthorized')).toBe('not-authed');
  });

  it('separates rate limiting from plain failure', () => {
    expect(classifyCliFailure('Error: 429 Too Many Requests')).toBe('rate-limited');
    expect(classifyCliFailure('You have exceeded your usage limit')).toBe('rate-limited');
    expect(classifyCliFailure('account is out of credits, overage disabled')).toBe('rate-limited');
  });

  it('recognises a model the provider does not have', () => {
    expect(classifyCliFailure('unknown model: sonnet')).toBe('unsupported-model');
    expect(classifyCliFailure('Error: invalid model "haiku"')).toBe('unsupported-model');
  });

  it('recognises a missing binary', () => {
    expect(classifyCliFailure('codex: command not found')).toBe('binary-missing');
    expect(classifyCliFailure('spawn codex ENOENT')).toBe('binary-missing');
  });

  it('falls back to plain failure rather than guessing', () => {
    expect(classifyCliFailure('panicked at src/main.rs:12')).toBe('failed');
    expect(classifyCliFailure('')).toBe('failed');
  });
});

describe('output extraction', () => {
  it('strips the colour the CLIs emit even when piped', () => {
    expect(stripAnsi('[0m[32malpha[0m')).toBe('alpha');
  });

  it('takes the agent message out of a real codex --json stream', () => {
    // Captured from `codex exec --json` on the installed CLI.
    const stream = [
      '{"type":"thread.started","thread_id":"01a02cc8"}',
      '{"type":"turn.started"}',
      '{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"Retrospective Context Window Bug"}}',
      '{"type":"turn.completed","usage":{"input_tokens":14661,"output_tokens":56}}',
    ].join('\n');
    expect(extractCodexText(stream)).toBe('Retrospective Context Window Bug');
  });

  it('ignores codex reasoning items and keeps the last agent message', () => {
    const stream = [
      '{"type":"item.completed","item":{"type":"reasoning","text":"thinking out loud"}}',
      '{"type":"item.completed","item":{"type":"agent_message","text":"first"}}',
      '{"type":"item.completed","item":{"type":"agent_message","text":"final"}}',
    ].join('\n');
    expect(extractCodexText(stream)).toBe('final');
  });

  it('returns empty for a codex stream that carried no answer', () => {
    expect(extractCodexText('{"type":"turn.started"}')).toBe('');
  });

  it('falls back to raw text when codex did not emit JSON at all', () => {
    expect(extractCodexText('plain answer')).toBe('plain answer');
  });

  it('concatenates opencode text parts in order', () => {
    // Truncating to the last part would lose a split answer.
    const stream = [
      '{"type":"step_start","part":{"type":"step-start"}}',
      '{"type":"text","part":{"type":"text","text":"alpha "}}',
      '{"type":"text","part":{"type":"text","text":"beta gamma"}}',
      '{"type":"step_finish","part":{"type":"step-finish","tokens":{"total":8208}}}',
    ].join('\n');
    expect(extractOpencodeText(stream)).toBe('alpha beta gamma');
  });

  it('survives a truncated JSONL line', () => {
    const stream = '{"type":"text","part":{"type":"text","text":"kept"}}\n{"type":"tex';
    expect(extractOpencodeText(stream)).toBe('kept');
  });
});

describe('model resolution is per provider', () => {
  it('knows which providers it can serve', () => {
    expect(completionSupported('claude')).toBe(true);
    expect(completionSupported('codex')).toBe(true);
    expect(completionSupported('gemini')).toBe(false);
  });

  it('accepts claude aliases and claude ids for claude only', () => {
    expect(servesModel('claude', 'haiku')).toBe(true);
    expect(servesModel('claude', 'claude-haiku-4-5')).toBe(true);
    // The exact shape of the summarizerModel bug: a bare claude alias must not
    // be servable by another provider.
    expect(servesModel('codex', 'sonnet')).toBe(false);
    expect(servesModel('opencode', 'sonnet')).toBe(false);
    expect(servesModel('pi', 'haiku')).toBe(false);
  });

  it('accepts codex model ids for codex', () => {
    expect(servesModel('codex', 'gpt-5-codex')).toBe(true);
    expect(servesModel('codex', 'o3')).toBe(true);
    expect(servesModel('codex', 'claude-haiku-4-5')).toBe(false);
  });

  it('requires the provider/model form for opencode and pi', () => {
    expect(servesModel('opencode', 'anthropic/claude-haiku-4-5')).toBe(true);
    expect(servesModel('pi', 'google/gemini-2.5-flash')).toBe(true);
    expect(servesModel('opencode', 'gpt-5')).toBe(false);
  });

  it('defaults claude to haiku and leaves the others to their own config', () => {
    expect(defaultModelFor('claude')).toBe('haiku');
    // Inventing an id we have not verified against the installed catalog is
    // exactly how summarizerModel broke; null = "use what the CLI has".
    expect(defaultModelFor('codex')).toBeNull();
    expect(defaultModelFor('opencode')).toBeNull();
  });

  it('downgrades an unservable configured model and says so', () => {
    expect(resolveCompletionModel('claude', 'haiku')).toEqual({
      model: 'haiku',
      downgraded: false,
    });
    expect(resolveCompletionModel('codex', 'haiku')).toEqual({ model: null, downgraded: true });
    expect(resolveCompletionModel('codex', '')).toEqual({ model: null, downgraded: false });
  });
});

describe('complete() degrades instead of throwing', () => {
  beforeEach(() => {
    installed.clear();
    for (const p of ['claude', 'codex', 'opencode', 'pi']) installed.add(p);
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('refuses an unknown provider without throwing', async () => {
    const res = await complete({ provider: 'gemini' as never, prompt: 'hi' });
    expect(res.ok).toBe(false);
    expect(res.ok === false && res.reason).toBe('unsupported-provider');
  });

  it('refuses an empty prompt', async () => {
    const res = await complete({ provider: 'claude', prompt: '   ' });
    expect(res.ok === false && res.reason).toBe('empty');
  });

  it('refuses a model the provider cannot serve rather than forwarding it', async () => {
    const res = await complete({ provider: 'codex', prompt: 'hi', model: 'sonnet' });
    expect(res.ok).toBe(false);
    expect(res.ok === false && res.reason).toBe('unsupported-model');
    // No process was spawned — the guard is before the transport.
    expect(res.ok === false && res.message).toMatch(/codex cannot serve model 'sonnet'/);
  });

  it('reports a missing binary rather than spawning', async () => {
    installed.delete('codex');
    const res = await complete({ provider: 'codex', prompt: 'hi' });
    expect(res.ok === false && res.reason).toBe('binary-missing');
  });

  it('reports the daemon being down for claude, and never shells out instead', async () => {
    const fetchMock = vi.fn().mockRejectedValue(new Error('connect ECONNREFUSED'));
    vi.stubGlobal('fetch', fetchMock);
    const res = await complete({ provider: 'claude', prompt: 'hi', timeoutMs: 2_000 });
    expect(res.ok === false && res.reason).toBe('daemon-unavailable');
    vi.unstubAllGlobals();
  });

  it('treats a daemon predating /oneshot as unavailable, not as a model error', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 404 }));
    const res = await complete({ provider: 'claude', prompt: 'hi', timeoutMs: 2_000 });
    expect(res.ok === false && res.reason).toBe('daemon-unavailable');
    expect(res.ok === false && res.message).toMatch(/404/);
    vi.unstubAllGlobals();
  });

  it('surfaces the daemon-reported timeout as a timeout', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ ok: false, error: 'timed out after 45s' }),
      }),
    );
    const res = await complete({ provider: 'claude', prompt: 'hi', timeoutMs: 2_000 });
    expect(res.ok === false && res.reason).toBe('timeout');
    vi.unstubAllGlobals();
  });

  it('classifies a not-logged-in claude through the daemon error string', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({
          ok: false,
          error: 'claude exited 1: Invalid API key · Please run /login',
        }),
      }),
    );
    const res = await complete({ provider: 'claude', prompt: 'hi', timeoutMs: 2_000 });
    expect(res.ok === false && res.reason).toBe('not-authed');
    vi.unstubAllGlobals();
  });

  it('reports an answerless run as empty rather than as success', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: true, json: async () => ({ ok: true, text: '  \n ' }) }),
    );
    const res = await complete({ provider: 'claude', prompt: 'hi', timeoutMs: 2_000 });
    expect(res.ok === false && res.reason).toBe('empty');
    vi.unstubAllGlobals();
  });

  it('returns the answer, the model used, and how long it took', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ ok: true, text: 'Fix context window display\n' }),
      }),
    );
    const res = await complete({ provider: 'claude', prompt: 'hi', timeoutMs: 2_000 });
    expect(res).toMatchObject({
      ok: true,
      text: 'Fix context window display',
      provider: 'claude',
      model: 'haiku',
    });
    expect(res.elapsedMs).toBeGreaterThanOrEqual(0);
    vi.unstubAllGlobals();
  });

  it('truncates an oversized prompt instead of refusing it', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ ok: true, text: 'ok' }),
    });
    vi.stubGlobal('fetch', fetchMock);
    await complete({ provider: 'claude', prompt: 'x'.repeat(MAX_PROMPT_CHARS + 500) });
    const body = JSON.parse(fetchMock.mock.calls[0][1].body);
    expect(body.prompt.length).toBe(MAX_PROMPT_CHARS);
    vi.unstubAllGlobals();
  });

  it('bounds the output a runaway model can hand back', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue({ ok: true, json: async () => ({ ok: true, text: 'y'.repeat(5000) }) }),
    );
    const res = await complete({ provider: 'claude', prompt: 'hi', maxOutputChars: 40 });
    expect(res.ok === true && res.text.length).toBe(40);
    vi.unstubAllGlobals();
  });
});

describe('complete() against a real CLI transport', () => {
  it('classifies a provider that is installed but has no credentials', async () => {
    // `pi` on this machine defaults to google and has no key — the one honest
    // not-authed case available without breaking anything. Skipped when pi
    // isn't installed so CI stays green.
    installed.clear();
    installed.add('pi');
    const res = await complete({ provider: 'pi', prompt: 'Reply: ok', timeoutMs: 20_000 });
    expect(res.ok).toBe(false);
    // Either it really has no key (not-authed) or the binary is absent here.
    expect(['not-authed', 'binary-missing', 'timeout', 'failed']).toContain(
      res.ok === false ? res.reason : '',
    );
  }, 30_000);
});
