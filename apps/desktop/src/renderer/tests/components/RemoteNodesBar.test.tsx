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
 *  - Waking spends money, so the control has to say so.
 *  - AND THE SLEEP HALF: a connected machine is BILLING, so its off switch has
 *    to be reachable — but only for a caller who could actually press it, so a
 *    view/triage phone still sees exactly what it saw before. Stopping ends the
 *    work on the machine, so its confirm copy names the WORK, not the saving.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act, waitFor, fireEvent, cleanup, within } from '@testing-library/react';
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
let nodesSleep: ReturnType<typeof vi.fn>;

function node(over: Partial<RemoteNodeView> = {}): RemoteNodeView {
  return { id: 'den', label: 'Fly node (den)', state: 'stopped', wakeable: true, ...over };
}

function installApi(snapshot: RemoteNodesSnapshot | null | (() => Promise<unknown>)) {
  hubEventCbs = [];
  nodesList = vi.fn(typeof snapshot === 'function' ? snapshot : async () => snapshot);
  nodesWake = vi.fn(async (id: string) => ({ ok: true, node: node({ id, state: 'waking' }) }));
  nodesSleep = vi.fn(async (id: string) => ({ ok: true, node: node({ id, state: 'stopping' }) }));
  (window as any).electronAPI = {
    nodesList,
    nodesWake,
    nodesSleep,
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

    // …and once it lands, the WARNING stands down. What is left is an off
    // switch, because a connected machine is billing and this caller can stop
    // it — before the sleep path there was nothing to offer and the row simply
    // disappeared.
    pushStateChange(node({ state: 'available', wakeable: true }), 'waking');
    await waitFor(() =>
      expect(screen.getByTestId('remote-node-den').getAttribute('data-node-state')).toBe(
        'available',
      ),
    );
    expect(screen.getByTestId('node-sleep-den')).toBeEnabled();
    expect(screen.queryByRole('button', { name: /^connect$/i })).toBeNull();
    // Seeded once; every later reading came off the event.
    expect(nodesList).toHaveBeenCalledTimes(1);
  });

  // …and for a caller who could NOT act on it, the old behaviour exactly: a
  // healthy machine is not news, and a permanent "all good" strip is chrome
  // nobody asked for.
  it('stands down completely for a caller with no authority to stop it', async () => {
    await mount({ nodes: [node()], canWake: false });
    await screen.findByTestId('remote-node-den');
    pushStateChange(node({ state: 'available', wakeable: true }), 'waking');
    await waitFor(() => expect(screen.queryByTestId('remote-node-den')).toBeNull());
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
    // A single tap opens the confirm step — it does not fire the wake yet.
    expect(nodesWake).not.toHaveBeenCalled();
    // Confirming is what actually spends the money.
    const confirmBtn = await screen.findByRole('menuitem', { name: /connect/i });
    await act(async () => {
      fireEvent.click(confirmBtn);
    });
    expect(nodesWake).toHaveBeenCalledWith('den');
  });

  it('does not wake on a tap alone — a single click only opens the confirm step', async () => {
    await mount({ nodes: [node()], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    const btn = row.querySelector('button')!;
    await act(async () => {
      fireEvent.click(btn);
    });
    expect(nodesWake).not.toHaveBeenCalled();
  });

  it('names the consequence in the confirm step, reusing the cost note', async () => {
    await mount({ nodes: [node()], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    await act(async () => {
      fireEvent.click(row.querySelector('button')!);
    });
    expect(await screen.findByRole('menu')).toHaveTextContent(/bills from boot/i);
  });

  it('cancelling the confirm step starts nothing', async () => {
    await mount({ nodes: [node()], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    await act(async () => {
      fireEvent.click(row.querySelector('button')!);
    });
    const cancelBtn = await screen.findByRole('menuitem', { name: /cancel/i });
    await act(async () => {
      fireEvent.click(cancelBtn);
    });
    expect(nodesWake).not.toHaveBeenCalled();
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('surfaces a refused wake instead of swallowing it', async () => {
    await mount({ nodes: [node()], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    nodesWake.mockResolvedValueOnce({ ok: false, error: 'the cloud API is rate-limiting' });
    await act(async () => {
      fireEvent.click(row.querySelector('button')!);
    });
    await act(async () => {
      fireEvent.click(await screen.findByRole('menuitem', { name: /connect/i }));
    });
    await waitFor(() =>
      expect(screen.getByTestId('remote-node-den')).toHaveTextContent(/rate-limiting/i),
    );
  });
});

describe('RemoteNodesBar — a draining machine cannot be woken by accident', () => {
  it('renders a dead Connect button, with its reason, while the node is stopping', async () => {
    // The hub ACCEPTS a wake mid-drain, so one click here would cancel a
    // shutdown somebody just asked for and start paying again. The row has to
    // refuse it, and say why where a phone can read it.
    await mount({
      nodes: [node({ state: 'stopping', mayBeRunning: true })],
      canWake: true,
    });
    const row = await screen.findByTestId('remote-node-den');
    const connect = within(row).getByRole('button', { name: /^connect$/i });
    expect(connect).toBeDisabled();
    expect(row).toHaveTextContent(/shutting down/i);

    fireEvent.click(connect);
    expect(nodesWake).not.toHaveBeenCalled();
    expect(screen.queryByRole('menuitem', { name: /connect/i })).toBeNull();
  });
});

describe('RemoteNodesBar — the honest costs', () => {
  // This assertion changed with the sleep path, and the change is the feature.
  // It used to require the row to say the machine was still running and
  // billing, because it was: the hub had no stop verb. The hub now stops what
  // its own wake started, so the row reports the FAILURE and leaves the bill to
  // the hub's own `detail` — which is the only thing that knows whether that
  // stop worked.
  it('reports failed wakes without claiming a bill the hub has since closed', async () => {
    await mount({ nodes: [node({ state: 'unreachable', wakeFailures: 2 })], canWake: true });
    const row = await screen.findByTestId('remote-node-den');
    expect(row).toHaveTextContent(/2 wakes failed/i);
    expect(row).not.toHaveTextContent(/wakes failed[^]*running and billing/i);
    expect(row).toHaveTextContent(/check its boot log/i);
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

  // THIS RULE NARROWED WITH THE SLEEP PATH, deliberately. "Nothing to report is
  // rendered as nothing" still holds — what changed is that a CONNECTED machine
  // is not nothing to report once there is something to do about it: it is
  // billing, and the off switch has to live somewhere. So the strip stays silent
  // for every caller who could not press it, which is every phone tier and every
  // node this hub merely observes.
  it('says nothing at all about a connected node this caller cannot stop', async () => {
    await mount({ nodes: [node({ state: 'available' })], canWake: false });
    expect(nodesList).toHaveBeenCalled();
    expect(screen.queryByTestId('remote-node-den')).toBeNull();
  });

  it('says nothing about a connected node the hub holds no credentials for', async () => {
    await mount({
      nodes: [node({ state: 'available', wakeable: false })],
      canWake: true,
    });
    expect(screen.queryByTestId('remote-node-den')).toBeNull();
  });
});

describe('RemoteNodesBar — the off switch', () => {
  it('offers a shutdown for a connected machine, behind a confirm that names the work', async () => {
    await mount({ nodes: [node({ state: 'available' })], canWake: true });
    const btn = await screen.findByTestId('node-sleep-den');
    expect(btn).toBeEnabled();

    await act(async () => {
      fireEvent.click(btn);
    });
    // The confirm names what STOPS, not what it saves — the money is why
    // somebody presses it, the work is what they need warning about.
    expect(await screen.findByText(/anything still running on it stops/i)).toBeTruthy();
    await act(async () => {
      fireEvent.click(await screen.findByRole('menuitem', { name: /shut down/i }));
    });
    expect(nodesSleep).toHaveBeenCalledWith('den');
    // ONLY an id crosses the seam. The signal and the drain window are the
    // hub's, and a renderer that could name the signal could name SIGKILL.
    expect(nodesSleep.mock.calls[0]).toHaveLength(1);
    await waitFor(() =>
      expect(screen.getByTestId('remote-node-den').getAttribute('data-node-state')).toBe(
        'stopping',
      ),
    );
  });

  it('never fires a shutdown without the confirm step', async () => {
    await mount({ nodes: [node({ state: 'available' })], canWake: true });
    await act(async () => {
      fireEvent.click(await screen.findByTestId('node-sleep-den'));
    });
    expect(nodesSleep).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.click(await screen.findByRole('menuitem', { name: /cancel/i }));
    });
    expect(nodesSleep).not.toHaveBeenCalled();
  });

  // NEVER RENDER A CONTROL THE BUS IS CERTAIN TO REFUSE. nodes.sleep is
  // host-authority only, so a view/triage phone sees the state and gets a
  // disabled control WITH THE REASON BESIDE IT — there is no hover on a phone.
  it('disables the shutdown for a caller without the authority, and says why', async () => {
    await mount({ nodes: [node({ state: 'available' })], canWake: false });
    // canWake:false is the same hub gate, so the strip renders nothing at all
    // for a connected node — exactly what a phone saw before this feature.
    expect(screen.queryByTestId('remote-node-den')).toBeNull();
  });

  it('disables the shutdown for a machine this hub holds no credentials for', async () => {
    await mount({
      nodes: [node({ state: 'unreachable', wakeable: false, mayBeRunning: true })],
      canWake: true,
    });
    const btn = await screen.findByTestId('node-sleep-den');
    expect(btn).toBeDisabled();
    expect(screen.getByTestId('remote-node-den')).toHaveTextContent(/no credentials on this hub/i);
    expect(nodesSleep).not.toHaveBeenCalled();
  });

  // THE CASE WITH A METER ATTACHED. A machine the hub says may still be running
  // is the precise reason this button exists — it is the failed wake that used
  // to bill forever.
  it('offers a shutdown for an unreachable machine the hub says may still be running', async () => {
    await mount({
      nodes: [
        node({
          state: 'unreachable',
          mayBeRunning: true,
          detail: 'the machine is running but its provider has not registered with the hub',
        }),
      ],
      canWake: true,
    });
    expect(await screen.findByTestId('node-sleep-den')).toBeEnabled();
  });

  // …and NOT for the machine the hub has already switched off, whose sentence
  // contains the word "billing". That reading is exactly what the first version
  // of this — a regex over the hub's prose — got backwards.
  it('offers no shutdown for a machine the hub already stopped, however its sentence reads', async () => {
    await mount({
      nodes: [
        node({
          state: 'unreachable',
          detail: 'the hub STOPPED IT AGAIN rather than leave it billing — check the boot log',
        }),
      ],
      canWake: true,
    });
    await screen.findByTestId('remote-node-den');
    expect(screen.queryByTestId('node-sleep-den')).toBeNull();
  });

  // A booting machine is mayBeRunning too. It gets the transition and nothing
  // else — one control per row, and it is the one describing what is happening.
  it('offers no shutdown for a machine that is still starting', async () => {
    await mount({
      nodes: [node({ state: 'waking', mayBeRunning: true })],
      canWake: true,
    });
    await screen.findByTestId('remote-node-den');
    expect(screen.queryByTestId('node-sleep-den')).toBeNull();
    expect(screen.getByTestId('remote-node-den')).toHaveTextContent(/starting/i);
  });

  it('offers no shutdown for a machine that is already off', async () => {
    await mount({ nodes: [node({ state: 'stopped' })], canWake: true });
    await screen.findByTestId('remote-node-den');
    expect(screen.queryByTestId('node-sleep-den')).toBeNull();
  });

  it('renders a refused shutdown as a reason on the row, not a crash', async () => {
    await mount({ nodes: [node({ state: 'available' })], canWake: true });
    nodesSleep.mockResolvedValueOnce({ ok: false, error: 'the cloud API is rate-limiting' });
    await act(async () => {
      fireEvent.click(await screen.findByTestId('node-sleep-den'));
    });
    await act(async () => {
      fireEvent.click(await screen.findByRole('menuitem', { name: /shut down/i }));
    });
    await waitFor(() =>
      expect(screen.getByTestId('remote-node-den')).toHaveTextContent(/rate-limiting/i),
    );
  });

  // An older preload has no nodesSleep. The whole feature uses one shape for
  // "not there" — render nothing — and this must not be the exception.
  it('renders nothing extra when the backend has no sleep verb', async () => {
    installApi({ nodes: [node({ state: 'available' })], canWake: true });
    delete (window as any).electronAPI.nodesSleep;
    render(<RemoteNodesBar />);
    await act(async () => {});
    expect(screen.queryByTestId('remote-node-den')).toBeNull();
  });
});
