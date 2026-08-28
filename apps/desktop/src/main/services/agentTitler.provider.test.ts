import { describe, it, expect, vi, beforeEach } from 'vitest';

/**
 * Titling used to shell out to the claude binary unconditionally, so a
 * codex-primary user with no claude installed silently lost auto-titling. These
 * pin the fix: the agent's OWN provider answers, the claude-shaped
 * `agents.autoTitle.model` default is resolved against that provider instead of
 * being forwarded, and every way the call can fail still produces a title.
 *
 * `sanitizeTitle`/`buildTitlePrompt` are pure and covered in agentTitler.test.ts;
 * this file is only the provider seam, so it needs the mocks that one avoids.
 */

const autoTitle = vi.hoisted(
  () =>
    ({ enabled: true, model: 'haiku' }) as {
      enabled?: boolean;
      model?: string;
      models?: Record<string, string>;
    },
);
vi.mock('./configService', () => ({
  configService: { getConfig: () => ({ agents: { autoTitle } }) },
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./claudeResolver', () => ({ claudeBaseArgv: () => ['claude'] }));
vi.mock('./agentProviders', () => ({
  resolveAgentBinary: (p: string) => p,
  isAgentBinaryInstalled: () => true,
}));

// Only `complete` is faked — the model-vocabulary logic under test is the real
// one, so a change to it would break these rather than being papered over.
const complete = vi.hoisted(() => vi.fn());
vi.mock('./directCompletion', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./directCompletion')>()),
  complete,
}));

import { generateAgentTitle, titleModelFor } from './agentTitler';

const EXCHANGE = { userMessage: 'fix the flaky sidebar resize test', assistantReply: 'On it' };

beforeEach(() => {
  complete.mockReset();
  complete.mockResolvedValue({ ok: true, text: 'Fix flaky sidebar resize test' });
  autoTitle.enabled = true;
  autoTitle.model = 'haiku';
  autoTitle.models = undefined;
});

describe('titleModelFor', () => {
  it('keeps the configured model when the provider can serve it', () => {
    expect(titleModelFor('claude', 'haiku')).toBe('haiku');
    expect(titleModelFor('codex', 'gpt-5-codex')).toBe('gpt-5-codex');
  });

  it("falls back to the provider's own default rather than forwarding a claude alias", () => {
    // The `supervisor.summarizerModel: 'sonnet'` bug, in the shape it would
    // have taken here: 'haiku' is meaningless to codex.
    expect(titleModelFor('codex', 'haiku')).toBeNull();
    expect(titleModelFor('opencode', 'haiku')).toBeNull();
  });
});

describe('generateAgentTitle dispatches on the agent, not on claude', () => {
  it('titles a codex agent through codex, with a model codex can serve', async () => {
    await generateAgentTitle({ ...EXCHANGE, provider: 'codex' });
    expect(complete).toHaveBeenCalledTimes(1);
    const req = complete.mock.calls[0][0];
    expect(req.provider).toBe('codex');
    // NOT 'haiku' — the configured default was resolved away.
    expect(req.model).toBeNull();
  });

  it('defaults to claude for agents spawned before the parameter existed', async () => {
    await generateAgentTitle(EXCHANGE);
    expect(complete.mock.calls[0][0]).toMatchObject({ provider: 'claude', model: 'haiku' });
  });

  it('bounds the call so a caller can never hang on it', async () => {
    await generateAgentTitle(EXCHANGE);
    const req = complete.mock.calls[0][0];
    expect(req.timeoutMs).toBeGreaterThan(0);
    expect(req.maxOutputChars).toBeGreaterThan(0);
  });

  it('returns the sanitized model title on success', async () => {
    complete.mockResolvedValue({ ok: true, text: '"Fix the flaky sidebar test"' });
    expect(await generateAgentTitle(EXCHANGE)).toBe('Fix the flaky sidebar test');
  });

  it.each([
    'binary-missing',
    'daemon-unavailable',
    'not-authed',
    'rate-limited',
    'timeout',
    'unsupported-model',
    'empty',
    'failed',
  ])('still returns a title when the call fails with %s', async (reason) => {
    complete.mockResolvedValue({ ok: false, reason, message: reason });
    const title = await generateAgentTitle(EXCHANGE);
    // The user's own first line — what RECENT has always shown.
    expect(title).toBe('fix the flaky sidebar resize test');
  });

  it('falls back rather than naming an agent after a refusal', async () => {
    complete.mockResolvedValue({ ok: true, text: "I'm sorry, I can't help with that." });
    expect(await generateAgentTitle(EXCHANGE)).toBe('fix the flaky sidebar resize test');
  });

  it('never lets a thrown error reach the caller', async () => {
    // Belt and braces: `complete` is documented never to throw, but the titler
    // sits in a UI path and must not care if that ever stops being true.
    complete.mockRejectedValue(new Error('boom'));
    expect(await generateAgentTitle(EXCHANGE)).toBe('fix the flaky sidebar resize test');
  });

  it('leaves the name alone when the feature is off, without calling out', async () => {
    autoTitle.enabled = false;
    expect(await generateAgentTitle({ ...EXCHANGE, provider: 'codex' })).toBeNull();
    expect(complete).not.toHaveBeenCalled();
  });

  it('does not call out for an empty opener', async () => {
    expect(await generateAgentTitle({ userMessage: '   ' })).toBeNull();
    expect(complete).not.toHaveBeenCalled();
  });
});

/**
 * The per-harness map, which is what lets a codex-primary user actually PICK a
 * codex title model instead of only being downgraded off the claude default.
 *
 * Unlike the supervisor's map this is not a memory of one picker: every agent is
 * titled by its OWN harness, so several entries have to be live at once.
 */
describe('agents.autoTitle.models — a title model per harness', () => {
  it('uses this harness’s entry over the legacy claude-shaped field', async () => {
    autoTitle.models = { codex: 'gpt-5' };
    await generateAgentTitle({ ...EXCHANGE, provider: 'codex' });
    expect(complete.mock.calls[0][0]).toMatchObject({ provider: 'codex', model: 'gpt-5' });
  });

  it('keeps every harness’s choice live at the same time', async () => {
    autoTitle.models = { claude: 'haiku', codex: 'gpt-5' };
    await generateAgentTitle({ ...EXCHANGE, provider: 'claude' });
    expect(complete.mock.calls[0][0]).toMatchObject({ provider: 'claude', model: 'haiku' });
    await generateAgentTitle({ ...EXCHANGE, provider: 'codex' });
    expect(complete.mock.calls[1][0]).toMatchObject({ provider: 'codex', model: 'gpt-5' });
  });

  it('a harness with no entry still refuses the claude-shaped legacy default', async () => {
    autoTitle.models = { claude: 'haiku' };
    await generateAgentTitle({ ...EXCHANGE, provider: 'codex' });
    // null = "let codex use the model the user already configured in that CLI".
    expect(complete.mock.calls[0][0]).toMatchObject({ provider: 'codex', model: null });
  });

  it('a wrong-harness id typed into the map is still caught by the adapter backstop', async () => {
    // The map is user-written, so someone can put a claude alias in the codex
    // row. resolveCompletionModel is the second layer that stops it.
    autoTitle.models = { codex: 'sonnet' };
    await generateAgentTitle({ ...EXCHANGE, provider: 'codex' });
    expect(complete.mock.calls[0][0]).toMatchObject({ provider: 'codex', model: null });
  });
});
