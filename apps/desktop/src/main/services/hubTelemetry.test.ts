/**
 * publishSnapshot is the hot one: it fires on every flush of every session
 * (~60/s while one streams) and reaches every bus client. It used to carry the
 * whole transcript each time, so a fleet of long-running sessions pushed
 * megabytes a second at the web/remote renderer.
 *
 * What has to hold: the payload is BOUNDED, it still carries the
 * `conversationOffset` anchor a client needs to splice it onto full history
 * (see mergeConversationWindow), and the remote-share gate still skips the work
 * entirely when nobody is listening.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

const publishToHub = vi.fn();
vi.mock('./hubClient', () => ({ publishToHub: (...a: unknown[]) => publishToHub(...a) }));

let remoteEnabled = true;
vi.mock('./hubDaemon', () => ({ isRemoteShareEnabled: () => remoteEnabled }));

import { publishSnapshot } from './hubTelemetry';

const bigSession = () =>
  ({
    sessionId: 's1',
    cwd: '/proj',
    status: 'running',
    conversation: Array.from({ length: 60 }, (_, i) => ({
      role: i % 2 === 0 ? 'user' : 'assistant',
      content: 'x'.repeat(8000),
    })),
    completedToolCalls: Array.from({ length: 40 }, (_, i) => ({
      id: `t${i}`,
      status: 'complete',
      completedAt: i,
      input: { blob: 'y'.repeat(8000) },
    })),
  }) as never;

beforeEach(() => {
  publishToHub.mockClear();
  remoteEnabled = true;
});

describe('publishSnapshot', () => {
  it('publishes a bounded window, not the whole transcript', () => {
    publishSnapshot(() => bigSession());
    expect(publishToHub).toHaveBeenCalledTimes(1);
    const { type, data } = publishToHub.mock.calls[0][0] as { type: string; data: any };
    expect(type).toBe('agent.snapshot');
    expect(data.conversation.length).toBeLessThan(60);
    expect(data.completedToolCalls.length).toBeLessThan(40);
  });

  // Without the anchor a client cannot tell WHERE the window belongs, and the
  // splice degrades to "replace history with 12 turns".
  it('carries the conversationOffset anchor for the turns it dropped', () => {
    publishSnapshot(() => bigSession());
    const { data } = publishToHub.mock.calls[0][0] as { data: any };
    // 60 turns in, 12 kept → 48 banked, half of them user sends.
    expect(data.conversationOffset).toBe(48);
    expect(data.conversationUserOffset).toBe(24);
    expect(data.conversationOffset + data.conversation.length).toBe(60);
  });

  it('truncates the payloads inside the turns it keeps', () => {
    publishSnapshot(() => bigSession());
    const { data } = publishToHub.mock.calls[0][0] as { data: any };
    expect(data.conversation[0].content.length).toBeLessThan(8000);
    expect(JSON.stringify(data).length).toBeLessThan(200_000);
  });

  // The factory exists so the `{...session}` copy is skipped when nothing is
  // listening; compaction must not have moved that work in front of the gate.
  it('does no work at all when remote sharing is off', () => {
    remoteEnabled = false;
    const make = vi.fn(() => bigSession());
    publishSnapshot(make);
    expect(make).not.toHaveBeenCalled();
    expect(publishToHub).not.toHaveBeenCalled();
  });
});
