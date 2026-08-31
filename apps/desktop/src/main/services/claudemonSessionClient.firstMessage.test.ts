/**
 * The spawn's FIRST MESSAGE, and the one property that matters about it: it
 * cannot be lost quietly.
 *
 * The prompt rides the spawn payload (`first_message`) so claudemon can queue
 * it inside its own spawn handler, before the id is visible to anyone. The
 * daemon then ACKNOWLEDGES it (`first_message_queued`). The acknowledgement is
 * the point — a daemon that predates the field answers an ordinary 200 with the
 * prompt nowhere, and an agent running with no instructions is
 * indistinguishable from a wedged one. So an unconfirmed spawn falls back to
 * the old two-call form, and a failure of THAT raises a banner instead of a
 * console line nobody reads.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

vi.mock('electron', () => ({
  BrowserWindow: class {},
  MessageChannelMain: class {},
  MessagePortMain: class {},
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:9999' }));
vi.mock('../lib/sseConsumer', () => ({ consumeSseStream: vi.fn() }));

const notifySystem = vi.fn();
vi.mock('./systemNotice', () => ({ notifySystem: (...a: unknown[]) => notifySystem(...a) }));

const { claudemonSessionClient } = await import('./claudemonSessionClient');

type Call = [string, RequestInit];

/** A fetch stub whose spawn answer is scripted, and which records every call. */
function stubFetch(spawnBody: Record<string, unknown>, messageOk = true) {
  const calls: Call[] = [];
  const fetchMock = vi.fn(async (url: string, init: RequestInit) => {
    calls.push([url, init]);
    if (url.endsWith('/message')) {
      return {
        ok: messageOk,
        status: messageOk ? 200 : 404,
        json: async () => ({ ok: messageOk }),
        text: async () => 'no wrapper attached to that session',
      } as unknown as Response;
    }
    return { ok: true, json: async () => spawnBody } as unknown as Response;
  });
  vi.stubGlobal('fetch', fetchMock);
  return calls;
}

const bodyOf = (init: RequestInit) => JSON.parse(init.body as string) as Record<string, unknown>;

beforeEach(() => {
  vi.restoreAllMocks();
  notifySystem.mockClear();
});

describe('spawnManaged — first message', () => {
  it('sends it as `first_message` on the spawn payload and needs no second call', async () => {
    const calls = stubFetch({ session_id: 'm-1', first_message_queued: true });

    await claudemonSessionClient.spawnManaged({
      provider: 'codex',
      cwd: '/proj',
      firstMessage: 'ship the thing',
    });

    expect(calls).toHaveLength(1);
    expect(calls[0][0]).toBe('http://127.0.0.1:9999/sessions/spawn-managed');
    expect(bodyOf(calls[0][1]).first_message).toBe('ship the thing');
    // Not `firstMessage` — the daemon payload is snake_case.
    expect(bodyOf(calls[0][1]).firstMessage).toBeUndefined();
  });

  it('falls back to POST /message when the daemon does not acknowledge it', async () => {
    const calls = stubFetch({ session_id: 'm-1' });

    await claudemonSessionClient.spawnManaged({
      provider: 'codex',
      cwd: '/proj',
      firstMessage: 'ship the thing',
    });

    expect(calls).toHaveLength(2);
    expect(calls[1][0]).toBe('http://127.0.0.1:9999/sessions/m-1/message');
    expect(bodyOf(calls[1][1]).text).toBe('ship the thing');
    expect(notifySystem).not.toHaveBeenCalled();
    // A successful fallback is a delivered message, so nothing is flagged.
    expect(claudemonSessionClient.takeUndeliveredFirstMessage('m-1')).toBe(false);
  });

  it('raises a banner naming the session when the prompt cannot be delivered at all', async () => {
    // The realistic shape of the failure: the daemon is old (no acknowledgement)
    // AND the follow-up lands in the window where a managed row is registered
    // but has no prompt channel yet — `404 no wrapper attached to that session`.
    stubFetch({ session_id: 'm-1' }, false);

    await claudemonSessionClient.spawnManaged({
      provider: 'codex',
      cwd: '/proj',
      firstMessage: 'ship the thing',
    });

    expect(notifySystem).toHaveBeenCalledTimes(1);
    const notice = notifySystem.mock.calls[0][0] as { level: string; detail: string };
    expect(notice.level).toBe('error');
    expect(notice.detail).toContain('m-1');
    // It must say what the user has to do, not just that something failed.
    expect(notice.detail).toContain('idle');
    // …and the spawn's ANSWER can say so, once — a banner is for the local
    // user, but the dispatcher that asked for this worker is somewhere else.
    expect(claudemonSessionClient.takeUndeliveredFirstMessage('m-1')).toBe(true);
    expect(
      claudemonSessionClient.takeUndeliveredFirstMessage('m-1'),
      'consumed once, so the set cannot grow',
    ).toBe(false);
  });

  it('a spawn with no first message makes exactly one call and no banner', async () => {
    const calls = stubFetch({ session_id: 'm-1' });
    await claudemonSessionClient.spawnManaged({ provider: 'codex', cwd: '/proj' });
    expect(calls).toHaveLength(1);
    expect(bodyOf(calls[0][1]).first_message).toBeUndefined();
    expect(notifySystem).not.toHaveBeenCalled();
  });

  it('serializes a managed model pair in snake_case beside the legacy model', async () => {
    const calls = stubFetch({ session_id: 'm-1' });
    await claudemonSessionClient.spawnManaged({
      provider: 'claude',
      cwd: '/proj',
      model: 'opus[1m]',
      modelIdentity: 'opus',
      contextWindow: 1_000_000,
    });
    expect(bodyOf(calls[0][1])).toMatchObject({
      model: 'opus[1m]',
      model_identity: 'opus',
      context_window: 1_000_000,
    });
  });
});

describe('spawn (PTY) — first message', () => {
  it('sends it as `first_message` on the spawn payload', async () => {
    const calls = stubFetch({ session_id: 'p-1', first_message_queued: true });

    await claudemonSessionClient.spawn({
      argv: ['claude'],
      cwd: '/proj',
      firstMessage: 'ship the thing',
    });

    expect(calls).toHaveLength(1);
    expect(calls[0][0]).toBe('http://127.0.0.1:9999/sessions/spawn');
    expect(bodyOf(calls[0][1]).first_message).toBe('ship the thing');
  });

  it('falls back when unacknowledged', async () => {
    const calls = stubFetch({ session_id: 'p-1' });
    await claudemonSessionClient.spawn({
      argv: ['claude'],
      cwd: '/proj',
      firstMessage: 'ship the thing',
    });
    expect(calls.map(([url]) => url)).toEqual([
      'http://127.0.0.1:9999/sessions/spawn',
      'http://127.0.0.1:9999/sessions/p-1/message',
    ]);
  });

  it('serializes a PTY model pair in snake_case beside the executable argv', async () => {
    const calls = stubFetch({ session_id: 'p-1' });
    await claudemonSessionClient.spawn({
      argv: ['claude', '--model', 'opus[1m]'],
      cwd: '/proj',
      model: 'opus[1m]',
      modelIdentity: 'opus',
      contextWindow: 1_000_000,
    });
    expect(bodyOf(calls[0][1])).toMatchObject({
      argv: ['claude', '--model', 'opus[1m]'],
      model: 'opus[1m]',
      model_identity: 'opus',
      context_window: 1_000_000,
    });
  });
});

describe('setModel — pair-aware managed switch', () => {
  it('posts the snake_case pair and maps the daemon-owned result', async () => {
    const fetchMock = vi.fn(
      async () =>
        ({
          ok: true,
          json: async () => ({
            model: 'opus[1m]',
            requested_selection: { model: 'opus', context_window: 1_000_000 },
          }),
        }) as Response,
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(
      claudemonSessionClient.setModel('s-1', 'opus[1m]', undefined, 'opus', 1_000_000),
    ).resolves.toEqual({
      ok: true,
      model: 'opus[1m]',
      requestedSelection: { model: 'opus', contextWindow: 1_000_000 },
    });
    expect(bodyOf(fetchMock.mock.calls[0][1] as RequestInit)).toEqual({
      model: 'opus[1m]',
      model_identity: 'opus',
      context_window: 1_000_000,
    });
  });
});
