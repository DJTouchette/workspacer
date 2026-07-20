/**
 * Attach-target verification: a restored pane can point at a session the
 * daemon holds as a resumable Stopped row, or at one that's gone entirely.
 * Both must surface terminal:exit (so the pane shows the session died), but
 * only the truly-gone case may tear the viewer stream down — a stopped row
 * revives under the SAME id on respawn, and the still-attached stream is what
 * brings the pane back to life without a remount.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

class FakePort {
  handlers: Record<string, (e: unknown) => void> = {};
  closed = false;
  on(event: string, cb: (e: unknown) => void) {
    this.handlers[event] = cb;
  }
  start() {}
  close() {
    this.closed = true;
  }
  postMessage() {}
}

const madeChannels: Array<{ port1: FakePort; port2: FakePort }> = [];

vi.mock('electron', () => ({
  BrowserWindow: class {},
  MessageChannelMain: class {
    port1 = new FakePort();
    port2 = new FakePort();
    constructor() {
      madeChannels.push(this as unknown as { port1: FakePort; port2: FakePort });
    }
  },
  MessagePortMain: class {},
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:9999' }));
vi.mock('../lib/sseConsumer', () => ({ consumeSseStream: vi.fn(async () => {}) }));

const { claudemonSessionClient } = await import('./claudemonSessionClient');

function fakeWindow() {
  const sent: Array<[string, unknown]> = [];
  const win = {
    isDestroyed: () => false,
    webContents: {
      postMessage: vi.fn(),
      send: (channel: string, payload: unknown) => {
        sent.push([channel, payload]);
      },
    },
  };
  return { win, sent };
}

/** Let the fire-and-forget verify fetch settle. */
const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  vi.restoreAllMocks();
  madeChannels.length = 0;
});

describe('claudemonSessionClient.attach — dead-target verification', () => {
  it('stopped target: fires terminal:exit but keeps the viewer stream open', async () => {
    const { win, sent } = fakeWindow();
    claudemonSessionClient.setMainWindow(win as never);
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => ({ mode: 'stopped' }),
      })),
    );

    claudemonSessionClient.attach('pane-1', 'sess-stopped');
    await flush();

    expect(sent).toContainEqual(['terminal:exit', 'pane-1']);
    // The port stays open: a respawn revives the same session id and the
    // attached SSE stream is the pane's only path back to live output.
    expect(madeChannels[0].port1.closed).toBe(false);
    // detach() only works on streams still in the map — proves it wasn't deleted.
    claudemonSessionClient.detach('pane-1');
    expect(madeChannels[0].port1.closed).toBe(true);
  });

  it('missing target (404): fires terminal:exit and tears the viewer down', async () => {
    const { win, sent } = fakeWindow();
    claudemonSessionClient.setMainWindow(win as never);
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: false,
        status: 404,
        json: async () => ({}),
      })),
    );

    claudemonSessionClient.attach('pane-2', 'sess-gone');
    await flush();

    expect(sent).toContainEqual(['terminal:exit', 'pane-2']);
    expect(madeChannels[0].port1.closed).toBe(true);
  });

  it('live target: no exit event, stream stays attached', async () => {
    const { win, sent } = fakeWindow();
    claudemonSessionClient.setMainWindow(win as never);
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => ({ mode: 'input' }),
      })),
    );

    claudemonSessionClient.attach('pane-3', 'sess-live');
    await flush();

    expect(sent.filter(([ch]) => ch === 'terminal:exit')).toHaveLength(0);
    expect(madeChannels[0].port1.closed).toBe(false);
    claudemonSessionClient.detach('pane-3');
  });
});
