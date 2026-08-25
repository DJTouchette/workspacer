/**
 * E2E for the /m Connect and Put-to-sleep buttons — the remote-node strip.
 *
 * Everything below the client is REAL: the hub binary, its node registry read
 * off a real `nodes.json`, its reconcile loop, its `nodes.list` / `nodes.wake`
 * handlers, the bus tier gate, and the `node.state_changed` topic. The only
 * fake is the Fly Machines API itself, which is an `http.Server` in this file
 * answering the three endpoints `internal/flyapi` calls. Nothing here has or
 * wants a real credential.
 *
 * What it pins (contract:
 * `.workspacer/reports/2026-08-24-fly-wake-contract.md`):
 *
 *  - a hub with NO registry shows nothing at all (the ordinary install);
 *  - `waking` renders as PROGRESS and is visibly distinct from `unreachable`;
 *  - a triage token sees the state and gets no live button;
 *  - a node the hub has no credential for gets a disabled button with a reason;
 *  - tapping Connect really reaches `nodes.wake`, which really starts the
 *    (fake) machine, and the state follows on the event — no polling;
 *  - AND THE SLEEP HALF: a machine the hub reports as RUNNING AND PROVIDING
 *    NOTHING is the case with a meter attached, and tapping Put to sleep really
 *    reaches `nodes.sleep`, which really stops the (fake) machine — with an
 *    explicit signal and drain window on the wire, because fly.toml's
 *    kill_timeout does not govern an API stop.
 */
import * as http from 'http';
import { test, expect, type Page } from '@playwright/test';
import { startMobileHub, HOST_TOKEN, TRIAGE_TOKEN, type MobileHub } from './fixtures/mobileHub';

const IPHONE = { width: 390, height: 844 };

/** The three Machines API endpoints `internal/flyapi` calls, and nothing else.
 *  `state` is what a GET reports; a start flips it to `started`. */
function startFakeFly(): Promise<{
  url: string;
  starts: string[];
  stops: Array<{ signal?: string; timeout?: string }>;
  setState(s: string): void;
  stop(): Promise<void>;
}> {
  let machineState = 'stopped';
  const starts: string[] = [];
  const stops: Array<{ signal?: string; timeout?: string }> = [];
  const srv = http.createServer((req, res) => {
    const url = req.url || '';
    res.setHeader('content-type', 'application/json');
    if (req.method === 'POST' && /\/start$/.test(url)) {
      starts.push(url);
      machineState = 'started';
      res.end(JSON.stringify({ ok: true }));
      return;
    }
    // The stop endpoint takes its OWN signal and timeout — fly.toml's
    // kill_signal / kill_timeout govern a PLATFORM stop and are never read
    // here — so the body is recorded and asserted rather than discarded.
    if (req.method === 'POST' && /\/stop$/.test(url)) {
      let body = '';
      req.on('data', (c) => {
        body += c;
      });
      req.on('end', () => {
        try {
          stops.push(JSON.parse(body || '{}'));
        } catch {
          stops.push({});
        }
        machineState = 'stopped';
        res.end(JSON.stringify({ ok: true }));
      });
      return;
    }
    if (/\/wait\?/.test(url)) {
      res.end(JSON.stringify({ ok: true }));
      return;
    }
    res.end(JSON.stringify({ id: 'm1', state: machineState }));
  });
  return new Promise((resolve) => {
    srv.listen(0, '127.0.0.1', () => {
      const port = (srv.address() as { port: number }).port;
      resolve({
        url: `http://127.0.0.1:${port}`,
        starts,
        stops,
        setState: (s) => {
          machineState = s;
        },
        stop: () =>
          new Promise<void>((r) => {
            srv.close(() => r());
          }),
      });
    });
  });
}

async function openFleet(page: Page, hub: MobileHub, token = HOST_TOKEN) {
  await page.setViewportSize(IPHONE);
  await page.goto(`${hub.url}/m?token=${token}`);
  await expect(page.locator('#title')).toHaveText('Fleet', { timeout: 10000 });
  await expect(page.locator('.agent').first()).toBeVisible({ timeout: 10000 });
}

// ── the ordinary install: no registry, and therefore no change at all ───────
test.describe('/m with no node registry', () => {
  let hub: MobileHub;
  test.beforeAll(async () => {
    hub = await startMobileHub();
  });
  test.afterAll(async () => {
    await hub?.stop();
  });

  test('renders nothing, and reports no error', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    await openFleet(page, hub);
    // `can('nodes.list')` is TRUE here — the method is in the view tier
    // unconditionally — so a client that gated on permission would have
    // rendered an empty or broken strip. The hub simply never registered it.
    await expect(page.locator('.node')).toHaveCount(0);
    // No toast either — a feature this hub does not have is not a failure to
    // report. (#toast is always in the DOM; it says nothing when nothing said.)
    await expect(page.locator('#toast')).toHaveText('');
    expect(errors).toEqual([]);
  });
});

// ── a hub that really does have machines ────────────────────────────────────
test.describe('/m with a node registry', () => {
  let hub: MobileHub;
  let fly: Awaited<ReturnType<typeof startFakeFly>>;

  test.beforeAll(async () => {
    fly = await startFakeFly();
    hub = await startMobileHub({
      nodes: [
        {
          id: 'den',
          label: 'Fly node (den)',
          fly: {
            app: 'wks-node-den',
            machineId: '17811944b12345',
            token: 'FlyV1 fm2_TEST_TOKEN_MUST_NEVER_REACH_A_CLIENT',
            baseUrl: fly.url,
          },
        },
        // No `fly` block at all: the hub can see this node and cannot start it.
        // Never `stopped` — without the cloud API it genuinely cannot tell "off
        // on purpose" from "broken" — so it reports unreachable, wakeable:false.
        { id: 'orphan', label: 'Orphan node' },
      ],
    });
  });
  test.afterAll(async () => {
    await hub?.stop();
    await fly?.stop();
  });

  test('renders the states honestly, and never leaks the Fly token', async ({ page }) => {
    await openFleet(page, hub);

    const den = page.locator('.node[data-node="den"]');
    const orphan = page.locator('.node[data-node="orphan"]');
    await expect(den).toBeVisible({ timeout: 10000 });
    await expect(orphan).toBeVisible();

    // The reconcile asked the (fake) cloud API and found the machine stopped.
    await expect(den).toHaveAttribute('data-node-state', 'stopped', { timeout: 10000 });
    await expect(den).toContainText(/asleep/i);
    // …and it is honest about what pressing the button costs.
    await expect(den).toContainText(/bills from boot/i);

    // The credential-less node is unreachable with the button explained, not
    // offered: a wake would fail every single time.
    await expect(orphan).toHaveAttribute('data-node-state', 'unreachable');
    await expect(orphan.locator('button')).toBeDisabled();
    await expect(orphan).toContainText(/no cloud credentials|no cloud coordinates/i);

    // The token lives in nodes.json and in the hub's memory. Nowhere else.
    expect(await page.content()).not.toContain('fm2_TEST_TOKEN');
  });

  test('a tap alone does not start the machine — it only opens the confirm', async ({ page }) => {
    await openFleet(page, hub);
    const den = page.locator('.node[data-node="den"]');
    await expect(den).toHaveAttribute('data-node-state', 'stopped', { timeout: 10000 });

    // Dismiss the confirm rather than accept it. window.confirm() blocks the
    // page's JS until answered, so the handler must be armed before the tap.
    page.once('dialog', (d) => d.dismiss());
    const before = fly.starts.length;
    await den.locator('button').click();
    await page.waitForTimeout(300);

    expect(fly.starts.length).toBe(before);
    await expect(den).toHaveAttribute('data-node-state', 'stopped');
  });

  test('Connect really starts the machine, and waking reads as progress', async ({ page }) => {
    await openFleet(page, hub);
    const den = page.locator('.node[data-node="den"]');
    await expect(den).toHaveAttribute('data-node-state', 'stopped', { timeout: 10000 });

    // A wake spends real money and this hub has no stop verb — confirming is
    // what actually reaches nodes.wake.
    page.once('dialog', (d) => d.accept());
    const before = fly.starts.length;
    await den.locator('button').click();

    // nodes.wake returns IMMEDIATELY with waking; the rest arrives on the
    // event. That is what makes a real waking state possible instead of a
    // spinner on a held request.
    await expect(den).toHaveAttribute('data-node-state', 'waking', { timeout: 10000 });
    expect(fly.starts.length).toBe(before + 1);

    // waking is PROGRESS, not a failure: its own word, the in-flight blue, and
    // a spinner that actually turns. The unreachable node beside it stays amber
    // with no spinner — which is the entire point of keeping them apart.
    await expect(den).toContainText(/starting/i);
    await expect(den).not.toContainText(/can't reach/i);
    await expect(den.locator('.spin')).toBeVisible();
    await expect(page.locator('.node[data-node="orphan"] .spin')).toHaveCount(0);
    // And it refuses to arm a second wake on a machine already starting.
    await expect(den.locator('button')).toBeDisabled();
  });
});

// The triage case gets its OWN hub: a node left `waking` by the wake test above
// correctly offers no button to ANY tier, which would make this vacuous.
test.describe('/m on a triage token', () => {
  let hub: MobileHub;
  let fly: Awaited<ReturnType<typeof startFakeFly>>;

  test.beforeAll(async () => {
    fly = await startFakeFly();
    hub = await startMobileHub({
      nodes: [
        {
          id: 'den',
          label: 'Fly node (den)',
          fly: {
            app: 'wks-node-den',
            machineId: '17811944b12345',
            token: 'FlyV1 fm2_TEST_TOKEN_MUST_NEVER_REACH_A_CLIENT',
            baseUrl: fly.url,
          },
        },
      ],
    });
  });
  test.afterAll(async () => {
    await hub?.stop();
    await fly?.stop();
  });

  test('sees the state and does NOT get the button', async ({ page }) => {
    await openFleet(page, hub, TRIAGE_TOKEN);
    const den = page.locator('.node[data-node="den"]');
    // nodes.list is VIEW tier, so the phone renders the state…
    await expect(den).toBeVisible({ timeout: 10000 });
    await expect(den).toHaveAttribute('data-node-state', 'stopped', { timeout: 10000 });
    await expect(den).toContainText(/asleep/i);
    // …and nodes.wake is host-authority only, so the control is disabled with
    // the reason beside it rather than dying on tap, and the machine is never
    // started.
    await expect(den.locator('button')).toBeDisabled();
    await expect(den).toContainText(/operator token/i);
    await den.locator('button').click({ force: true });
    await page.waitForTimeout(500);
    expect(fly.starts).toEqual([]);
  });
});

// ── the case with a meter attached ─────────────────────────────────────────
//
// A machine that is RUNNING and providing nothing is the shape a failed wake
// leaves behind, and before the sleep path it billed until somebody opened the
// cloud console. Its own hub, because the reconcile has to find the machine
// already `started`.
test.describe('/m against a machine that is running and providing nothing', () => {
  let hub: MobileHub;
  let fly: Awaited<ReturnType<typeof startFakeFly>>;

  test.beforeAll(async () => {
    fly = await startFakeFly();
    fly.setState('started'); // up, and no brain will ever dial in
    hub = await startMobileHub({
      nodes: [
        {
          id: 'den',
          label: 'Fly node (den)',
          fly: {
            app: 'wks-node-den',
            machineId: '17811944b12345',
            token: 'FlyV1 fm2_TEST_TOKEN_MUST_NEVER_REACH_A_CLIENT',
            baseUrl: fly.url,
          },
        },
      ],
    });
  });
  test.afterAll(async () => {
    await hub?.stop();
    await fly?.stop();
  });

  test('offers Put to sleep, and really stops the machine with an explicit signal', async ({
    page,
  }) => {
    await openFleet(page, hub);
    const den = page.locator('.node[data-node="den"]');
    // Running with no provider is UNREACHABLE — never `available`, and never
    // `stopped`. The cloud API saying `started` says nothing about whether
    // workspacer is running on the box.
    await expect(den).toHaveAttribute('data-node-state', 'unreachable', { timeout: 10000 });
    await expect(den).toContainText(/is running/i);

    const btn = den.locator('button');
    await expect(btn).toBeEnabled();
    await expect(btn).toContainText(/put to sleep/i);
    // The copy names the WORK, not the saving.
    await expect(den).toContainText(/anything still running on it stops/i);

    // A tap alone opens the confirm and stops nothing.
    page.once('dialog', (d) => d.dismiss());
    await btn.click();
    await page.waitForTimeout(300);
    expect(fly.stops).toEqual([]);

    page.once('dialog', (d) => d.accept());
    await btn.click();
    await expect(den).toHaveAttribute('data-node-state', 'stopped', { timeout: 15000 });

    // THE ONE API DETAIL THAT IS EXPENSIVE TO GET WRONG: a stop issued through
    // the Machines API never reads fly.toml, so the signal and the drain window
    // have to be on the wire. A stop without them SIGKILLs the node mid-flush,
    // and a SIGKILLed node writes no exit record for the next wake to read.
    expect(fly.stops.length).toBe(1);
    expect(fly.stops[0].signal).toBe('SIGTERM');
    expect(fly.stops[0].timeout).toBeTruthy();

    // …and the credential never went anywhere near the page.
    expect(await page.content()).not.toContain('fm2_TEST_TOKEN');
  });
});
