/**
 * SECURITY.md #9: the daemon signal endpoint must only ever receive a known
 * signal name. Both the `claude:signal` IPC handler and the `claude.signal` hub
 * capability funnel through claudemonSessionClient.signal(), so the allowlist
 * lives there — this suite pins that chokepoint.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

// The client imports Electron + the daemon URL + the SSE consumer at module load;
// stub them so we can exercise signal() in isolation.
vi.mock('electron', () => ({
  BrowserWindow: class {},
  MessageChannelMain: class {},
  MessagePortMain: class {},
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:9999' }));
vi.mock('../lib/sseConsumer', () => ({ consumeSseStream: vi.fn() }));

const { claudemonSessionClient } = await import('./claudemonSessionClient');

beforeEach(() => {
  vi.restoreAllMocks();
});

describe('claudemonSessionClient.signal — allowlist', () => {
  it('forwards each allowed signal to the daemon endpoint', async () => {
    const fetchMock = vi.fn(async () => ({ ok: true }) as Response);
    vi.stubGlobal('fetch', fetchMock);

    for (const sig of ['SIGTERM', 'SIGINT', 'SIGKILL', 'SIGSTOP', 'SIGCONT']) {
      await claudemonSessionClient.signal('sess-1', sig);
    }

    expect(fetchMock).toHaveBeenCalledTimes(5);
    const firstCall = fetchMock.mock.calls[0];
    expect(firstCall[0]).toBe('http://127.0.0.1:9999/sessions/sess-1/signal');
    expect(JSON.parse((firstCall[1] as RequestInit).body as string)).toEqual({ signal: 'SIGTERM' });
  });

  it('rejects an unknown signal without ever hitting the daemon', async () => {
    const fetchMock = vi.fn(async () => ({ ok: true }) as Response);
    vi.stubGlobal('fetch', fetchMock);

    for (const bad of ['SIGUSR1', 'sigterm', 'rm -rf', 'SIGTERM; SIGKILL', '9']) {
      await expect(claudemonSessionClient.signal('sess-1', bad)).rejects.toThrow(/unrecognized signal/);
    }
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
