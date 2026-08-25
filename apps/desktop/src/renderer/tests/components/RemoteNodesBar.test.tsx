/**
 * The remote-node strip: the Connect button, and the two ways it must NOT
 * appear.
 *
 * The contract these pin (`.workspacer/reports/2026-08-24-fly-wake-contract.md`):
 *
 *  - A hub with NO node registry answers `no provider for nodes.list`. That is
 *    a FEATURE-ABSENT signal, not an error — every existing install is in that
 *    state and must see no change whatsoever. Nothing renders. No toast.
 *  - `waking` is not `unreachable`. A machine takes real seconds to boot, and a
 *    state that looks identical to a hang is what makes someone give up.
 *  - `nodes.wake` is host-authority only, so a view/triage phone gets the STATE
 *    and NOT the button. Never render a control that will be refused.
 *  - Waking spends money and this hub has no way to stop a machine, so the
 *    control has to say so.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act, waitFor, fireEvent, cleanup } from '@testing-library/react';
import React from 'react';
import type { RemoteNodesSnapshot, RemoteNodeView } from '../../src/lib/remoteNodes';

const { RemoteNodesBar } = await import('../../src/components/RemoteNodesBar');

type HubEventCb = (ev: {
  id: string;
  type: string;
  source: string;
  time: string;
  data?: unknown;
}) => void;

let hubEventCbs: HubEventCb[] = [];
let nodesList: ReturnType<typeof vi.fn>;
let nodesWake: ReturnType<typeof vi.fn>;

function node(over: Partial<RemoteNodeView> = {}): RemoteNodeView {
  return { id: 'den', label: 'Fly node (den)', state: 'stopped', wakeable: true, ...over };
}

function installApi(snapshot: RemoteNodesSnapshot | null | (() => Promise<unknown>)) {
  hubEventCbs = [];
  nodesList = vi.fn(typeof snapshot === 'function' ? snapshot : async () => snapshot);
  nodesWake = vi.fn(async (id: string) => ({ ok: true, node: node({ id, state: 'waking' }) }));
  (window as any).electronAPI = {
    nodesList,
    nodesWake,
    onHubEvent: (cb: HubEventCb) => {
      hubEventCbs.push(cb);
      return () => {
        hubEventCbs = hubEventCbs.filter((c) => c !== cb);
      };
    },
    onHubStatus: () => () => {},
  };
}

/** Install the backend and mount the strip — every test needs both. */
async function mount(snapshot: RemoteNodesSnapshot | null | (() => Promise<unknown>)) {
  installApi(snapshot);
  const r = render(<RemoteNodesBar />);
  await act(async () => {});
  return r;
}

function pushStateChange(n: RemoteNodeView, previous: string) {
  act(() => {
    for (const cb of hubEventCbs)
      cb({
        id: 'e1',
        type: 'node.state_changed',
        source: 'nodes',
        time: '2026-08-24T21:00:00Z',
        data: { node: n, previous },
      });
  });
}

beforeEach(() => {
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  (window as any).electronAPI = undefined;
});

describe('RemoteNodesBar — a hub with no node registry', () => {
  it('renders NOTHING when nodes.list has no provider, and shows no error', async () => {
    const { container } = await mount(async () => {
      throw new Error('no provider for nodes.list');
    });
    expect(nodesList).toHaveBeenCalled();
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText(/no provider/i)).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('renders nothing on an older client with no nodesList at all', async () => {
    (window as any).electronAPI = { onHubEvent: () => () => {}, onHubStatus: () => () => {} };
    const { container } = render(<RemoteNodesBar />);
    await act(async () => {});
    expect(container).toBeEmptyDOMElement();
  });

  it('renders nothing for a registry that exists but is empty', async () => {
    const { container } = await mount({ nodes: [], canWake: true });
    expect(nodesList).toHaveBeenCalled();
    expect(container).toBeEmptyDOMElement();
  });
});

describe('RemoteNodesBar — waking is not unreachable', () => {
  it('renders a starting machine as progress, and an unreachable one as a warning', async () => {
    await mount({
      nodes: [
        node({ id: 'waker', label: 'waker', state: 'waking' }),
        node({ id: 'broken', label: 'broken', state: 'unreachable', wakeable: true }),
      ],
      canWake: true,
    });
    const waking = await screen.findByTestId('remote-node-waker');
    const dead = await screen.findByTestId('remote-node-broken');

    // Distinct words…
    expect(waking).toHaveTextContent(/starting/i);
    expect(dead).toHaveTextContent(/can't reach/i);
    expect(waking).not.toHaveTextContent(/can't reach/i);

    // …distinct machine-readable state…
    expect(waking.getAttribute('data-node-state')).toBe('waking');
    expect(dead.getAttribute('data-node-state')).toBe('unreachable');

    // …and distinct tone: waking uses the working/busy token, not the warning
    // one. A booting machine that paints like a failure is the whole bug.
    const wakingTone = waking.querySelector('[data-node-tone]')?.getAttribute('data-node-tone');
    const deadTone = dead.querySelector('[data-node-tone]')?.getAttribute('data-node-tone');
    expect(wakingTone).toBe('busy');
    expect(deadTone).toBe('warning');
    expect(wakingTone).not.toBe(deadTone);

    // A machine that is already starting must not offer a second wake.
    expect(waking.querySelector('button')).toBeDisabled();
  });

  it('follows a wake from stopped → waking → available on the event, without polling', async () => {
    await mount({ nodes: [node()], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    expect(row.getAttribute('data-node-state')).toBe('stopped');
    expect(row).toHaveTextContent(/asleep/i);

    pushStateChange(node({ state: 'waking', detail: 'the machine is up; waiting' }), 'stopped');
    await waitFor(() =>
      expect(screen.getByTestId('remote-node-den').getAttribute('data-node-state')).toBe('waking'),
    );

    // …and once it lands, the strip stands down: a healthy machine is not news.
    pushStateChange(node({ state: 'available', wakeable: true }), 'waking');
    await waitFor(() => expect(screen.queryByTestId('remote-node-den')).toBeNull());
    // Seeded once; every later reading came off the event.
    expect(nodesList).toHaveBeenCalledTimes(1);
  });
});

describe('RemoteNodesBar — never offer a button that will be refused', () => {
  it('shows the state but no live Connect on a view/triage tier', async () => {
    await mount({ nodes: [node()], canWake: false });
    const row = await screen.findByTestId('remote-node-den');
    expect(row).toHaveTextContent(/asleep/i);
    const btn = row.querySelector('button');
    expect(btn).toBeDisabled();
    expect(row).toHaveTextContent(/operator token/i);
    fireEvent.click(btn!);
    expect(nodesWake).not.toHaveBeenCalled();
  });

  it('disables Connect for a node the hub holds no credentials for', async () => {
    await mount({ nodes: [node({ wakeable: false, state: 'unreachable' })], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    expect(row.querySelector('button')).toBeDisabled();
    expect(row).toHaveTextContent(/no credentials/i);
  });

  it('calls nodes.wake and says what it costs', async () => {
    await mount({ nodes: [node()], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    // The bill is on the screen, not only in a tooltip.
    expect(row).toHaveTextContent(/bills from boot/i);
    const btn = row.querySelector('button')!;
    expect(btn).toBeEnabled();
    expect(btn).toHaveTextContent(/connect/i);
    await act(async () => {
      fireEvent.click(btn);
    });
    expect(nodesWake).toHaveBeenCalledWith('den');
  });

  it('surfaces a refused wake instead of swallowing it', async () => {
    await mount({ nodes: [node()], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    nodesWake.mockResolvedValueOnce({ ok: false, error: 'the cloud API is rate-limiting' });
    await act(async () => {
      fireEvent.click(row.querySelector('button')!);
    });
    await waitFor(() =>
      expect(screen.getByTestId('remote-node-den')).toHaveTextContent(/rate-limiting/i),
    );
  });
});

describe('RemoteNodesBar — the honest costs', () => {
  it('shows failed wakes as money still burning', async () => {
    await mount({ nodes: [node({ state: 'unreachable', wakeFailures: 2 })], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    expect(row).toHaveTextContent(/2 wakes failed/i);
    expect(row).toHaveTextContent(/running and billing/i);
  });

  it('shows a crash notice on a node that is otherwise fine', async () => {
    await mount({
      nodes: [
        node({
          state: 'available',
          lastExit: { reason: 'claudemon-died', exitCode: 1, at: '2026-08-24T21:00:00Z' },
        }),
      ],
      canWake: true,
    });
    const row = await screen.findByTestId('remote-node-den');
    expect(row).toHaveTextContent(/did not end cleanly/i);
    expect(row).toHaveTextContent(/claudemon-died/);
  });

  it('says nothing at all about a healthy connected node', async () => {
    await mount({ nodes: [node({ state: 'available' })], canWake: true });
    expect(nodesList).toHaveBeenCalled();
    // Nothing to report is rendered as nothing — a permanent "all good" strip
    // is chrome nobody asked for.
    expect(screen.queryByTestId('remote-node-den')).toBeNull();
  });
});
