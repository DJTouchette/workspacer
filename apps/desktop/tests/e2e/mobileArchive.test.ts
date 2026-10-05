/**
 * /m — the phone honours the hub's shared session archive.
 *
 * A session archived in the native client (or /app) must leave the phone's
 * fleet too, without being stopped or forgotten: the real hub binary owns the
 * archive (`sessionArchive.get/set` + `sessionArchive.changed`), a bare second
 * bus connection plays the native client, and the fake provider records every
 * call so "never stopped" is an assertion, not a hope.
 *
 * SAFETY: scratch state and ephemeral ports via `fixtures/mobileHub.ts`; no
 * model or provider is ever started.
 */
import { test, expect, type Page } from '@playwright/test';
import { startMobileHub, HOST_TOKEN, VIEW_TOKEN, type MobileHub } from './fixtures/mobileHub';

let hub: MobileHub;
const IPHONE = { width: 390, height: 844 };
// What archiving must never send.
const LIFECYCLE = ['claude.signal', 'agents.close', 'sessions.delete'];

/** A second bus client shaped like the native app. */
async function nativeClient(token = HOST_TOKEN) {
  const ws = new WebSocket(`${hub.url.replace('http', 'ws')}/bus?token=${token}`);
  await new Promise<void>((resolve, reject) => {
    ws.addEventListener('open', () => resolve());
    ws.addEventListener('error', () => reject(new Error('native socket failed')));
  });
  let seq = 0;
  const pending = new Map<string, { resolve: (v: any) => void; reject: (e: Error) => void }>();
  ws.addEventListener('message', (ev: MessageEvent) => {
    const f = JSON.parse(String(ev.data));
    const waiter = f.id && pending.get(String(f.id));
    if (!waiter) return;
    pending.delete(String(f.id));
    if (f.op === 'error') waiter.reject(new Error(String(f.error)));
    else waiter.resolve(f.result);
  });
  const call = (method: string, params: unknown) =>
    new Promise<any>((resolve, reject) => {
      const id = `native-${++seq}`;
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ op: 'call', id, method, params }));
    });
  return { call, close: () => ws.close() };
}

test.beforeAll(async () => {
  hub = await startMobileHub();
});
test.afterAll(async () => {
  await hub?.stop();
});
test.beforeEach(async () => {
  hub.reset();
  test.setTimeout(120_000);
  // The archive is hub state that outlives a spec: start each one clean.
  const native = await nativeClient();
  for (const sessionId of Object.keys((await native.call('sessionArchive.get', {})).archived))
    await native.call('sessionArchive.set', { sessionId, archived: false });
  native.close();
});

const card = (page: Page, id: string) => page.locator(`.agent[data-agent="${id}"]`);

async function openClient(page: Page, token = HOST_TOKEN) {
  await page.setViewportSize(IPHONE);
  await page.goto(`${hub.url}/m?token=${token}`);
  await expect(page.locator('#title')).toHaveText('Fleet', { timeout: 10_000 });
  await expect(card(page, 'ws1')).toBeVisible({ timeout: 10_000 });
}

test('a native archive leaves the phone fleet live and after reload, and Restore returns it', async ({
  page,
}) => {
  const native = await nativeClient();
  try {
    await openClient(page);
    await expect(card(page, 'ws2')).toBeVisible();

    await native.call('sessionArchive.set', { sessionId: 'ws2', archived: true });
    await expect(card(page, 'ws2')).toHaveCount(0, { timeout: 10_000 });
    await expect(card(page, 'ws1')).toBeVisible();

    // Reload: the fleet waits for the archive, so ws2 is absent the moment
    // the first card paints — never shown, then hidden.
    await page.reload();
    await page.locator('.agent').first().waitFor({ timeout: 10_000 });
    expect(await card(page, 'ws2').count()).toBe(0);

    // The Archived chip lists it, and only it.
    const chip = page.locator('#filters button[data-f="archived"]');
    await expect(chip).toContainText('1');
    await chip.click();
    await expect(card(page, 'ws2')).toBeVisible();
    await expect(card(page, 'ws1')).toHaveCount(0);
    await page
      .screenshot({ path: test.info().outputPath('m-archived-filter.png') })
      .catch(() => {});

    // Its conversation is intact and restoring is one row in ⋯.
    await card(page, 'ws2').locator('[data-open="ws2"]').click();
    await expect(page.locator('#title')).not.toHaveText('Fleet');
    await page.locator('#moreBtn').click();
    await page.getByRole('button', { name: 'Restore from archive' }).click();
    await expect
      .poll(async () => (await native.call('sessionArchive.get', {})).archived)
      .toEqual({});
    await page.locator('#backBtn').click();
    await expect(card(page, 'ws2')).toBeVisible({ timeout: 10_000 });

    for (const m of LIFECYCLE) expect(hub.callsTo(m), m).toEqual([]);
  } finally {
    native.close();
  }
});

test('archiving from the phone hides the card everywhere and never stops it', async ({ page }) => {
  const native = await nativeClient();
  try {
    await openClient(page);
    await card(page, 'ws2').locator('[data-open="ws2"]').click();
    await page.locator('#moreBtn').click();
    await page.getByRole('button', { name: 'Archive (hide from fleet)' }).click();
    await expect
      .poll(async () => (await native.call('sessionArchive.get', {})).archived.ws2 > 0)
      .toBe(true);
    await page.locator('#backBtn').click();
    await expect(card(page, 'ws2')).toHaveCount(0, { timeout: 10_000 });

    // A resumable (stopped) session archives the same way and restores from
    // its row in the Archived view.
    await native.call('sessionArchive.set', { sessionId: 'old-1', archived: true });
    await expect(page.locator('[data-resume="old-1"]')).toHaveCount(0, { timeout: 10_000 });
    await page.locator('#filters button[data-f="archived"]').click();
    await page.locator('[data-unarchive="old-1"]').click();
    await expect
      .poll(async () => (await native.call('sessionArchive.get', {})).archived['old-1'])
      .toBeUndefined();
    for (const m of LIFECYCLE) expect(hub.callsTo(m), m).toEqual([]);
  } finally {
    native.close();
  }
});

test('a view token sees the archive but cannot change it', async ({ page }) => {
  const native = await nativeClient();
  try {
    await native.call('sessionArchive.set', { sessionId: 'ws2', archived: true });
    await openClient(page, VIEW_TOKEN);
    expect(await card(page, 'ws2').count()).toBe(0);
    await page.locator('#filters button[data-f="archived"]').click();
    await expect(card(page, 'ws2')).toBeVisible();
    await card(page, 'ws2').locator('[data-open="ws2"]').click();
    await page.locator('#moreBtn').click();
    await expect(
      page.getByRole('button', { name: /Restore from archive|Archive \(hide/ }),
    ).toHaveCount(0);

    const viewer = await nativeClient(VIEW_TOKEN);
    try {
      await expect(viewer.call('sessionArchive.get', {})).resolves.toMatchObject({
        archived: { ws2: expect.any(Number) },
      });
      await expect(
        viewer.call('sessionArchive.set', { sessionId: 'ws2', archived: false }),
      ).rejects.toThrow();
    } finally {
      viewer.close();
    }
    expect((await native.call('sessionArchive.get', {})).archived.ws2).toBeGreaterThan(0);
  } finally {
    native.close();
  }
});

test('a delayed boot archive read cannot undo a newer pushed archive or flash its row', async ({
  page,
}) => {
  let release: (() => void) | undefined;
  let forwardedArchive = false;
  await page.routeWebSocket('**/bus?*', (socket) => {
    const server = socket.connectToServer();
    const reads = new Set<string>();
    socket.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (frame.method === 'sessionArchive.get') reads.add(String(frame.id));
      server.send(message);
    });
    server.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (reads.has(String(frame.id)) && frame.op === 'result') {
        release = () => socket.send(message);
        return;
      }
      if (frame.event?.type === 'sessionArchive.changed') forwardedArchive = true;
      socket.send(message);
    });
  });
  await page.addInitScript(() => {
    (window as any).sawArchivedRow = false;
    new MutationObserver(() => {
      if (document.querySelector('.agent[data-agent="ws2"]')) (window as any).sawArchivedRow = true;
    }).observe(document, { childList: true, subtree: true });
  });
  await page.setViewportSize(IPHONE);
  await page.goto(`${hub.url}/m?token=${HOST_TOKEN}`);
  await expect.poll(() => !!release).toBe(true);
  const native = await nativeClient();
  try {
    await native.call('sessionArchive.set', { sessionId: 'ws2', archived: true });
    await expect.poll(() => forwardedArchive).toBe(true);
    release!();
    await expect(card(page, 'ws1')).toBeVisible();
    await expect(card(page, 'ws2')).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).sawArchivedRow)).toBe(false);
  } finally {
    native.close();
  }
});

test('late replies from archive/restore/archive do not settle the newest optimistic choice', async ({
  page,
}) => {
  const releases: Array<() => void> = [];
  await page.routeWebSocket('**/bus?*', (socket) => {
    const server = socket.connectToServer();
    const writes = new Set<string>();
    socket.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (frame.method === 'sessionArchive.set') writes.add(String(frame.id));
      server.send(message);
    });
    server.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (frame.event?.type === 'sessionArchive.changed') return;
      if (writes.has(String(frame.id)) && frame.op === 'result') {
        releases.push(() => socket.send(message));
        return;
      }
      socket.send(message);
    });
  });
  await openClient(page);
  await card(page, 'ws2').locator('[data-open="ws2"]').click();
  for (const name of [
    'Archive (hide from fleet)',
    'Restore from archive',
    'Archive (hide from fleet)',
  ]) {
    await page.locator('#moreBtn').click();
    await page.getByRole('button', { name, exact: true }).click();
  }
  await expect.poll(() => releases.length).toBe(3);
  releases[0]();
  releases[1]();
  await page.locator('#moreBtn').click();
  await expect(
    page.getByRole('button', { name: 'Restore from archive', exact: true }),
  ).toBeVisible();
  releases[2]();
  for (const m of LIFECYCLE) expect(hub.callsTo(m), m).toEqual([]);
});
