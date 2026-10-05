/**
 * `/app` — archived sessions leave the web sidebar, wherever they were archived.
 *
 * The user archives in the native client and expects the web view to stop
 * listing those sessions while keeping them. Archive is hub-owned view state
 * (`sessionArchive.get/set` + `sessionArchive.changed`), so this drives the
 * REAL hub binary with the REAL web bundle, and plays the native client with a
 * second, bare bus connection making the same call native makes.
 *
 * Asserted beyond hiding: the archived workspace is still in the shared layout
 * (nothing forgotten), no stop/close/signal ever reaches the provider, a
 * reload keeps it hidden, the Archived view lists it, and Restore brings it
 * back — and a web archive reaches the "native" listener the same way.
 *
 * SAFETY: scratch state and ephemeral ports via `fixtures/appHub.ts`; the
 * provider is a fake, so no model is ever called.
 */
import { test, expect, type Page } from '@playwright/test';
import { startAppHub, layoutOf, workspace, pane, HOST_TOKEN, type AppHub } from './fixtures/appHub';

let hub: AppHub;

test.use({ screenshot: 'only-on-failure' });

test.beforeAll(async () => {
  hub = await startAppHub({
    layout: layoutOf(
      workspace('agent-keep', {
        name: 'keep-me',
        sessionId: 'ws1',
        panes: [pane('claude', { title: 'keep-me', transport: 'stream', attachSessionId: 'ws1' })],
      }),
      workspace('agent-archive', {
        name: 'archive-me',
        sessionId: 'ws2',
        panes: [
          pane('claude', { title: 'archive-me', transport: 'stream', attachSessionId: 'ws2' }),
        ],
      }),
    ),
  });
});

test.afterAll(async () => {
  await hub?.stop();
});

test.beforeEach(async () => {
  hub.reset();
  // Booting the full renderer twice (open + reload) needs more than 30s.
  test.setTimeout(240_000);
  // The archive is hub state that outlives a spec: start each one clean.
  const native = await nativeClient(new URL(hub.url).port);
  for (const sessionId of Object.keys((await native.call('sessionArchive.get', {})).archived))
    await native.call('sessionArchive.set', { sessionId, archived: false });
  native.close();
});

/** A second bus client shaped like the native app: host token, plain calls,
 *  and a subscription to the archive topic. */
async function nativeClient(port: string) {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/bus?token=${HOST_TOKEN}`);
  await new Promise<void>((resolve, reject) => {
    ws.addEventListener('open', () => resolve());
    ws.addEventListener('error', () => reject(new Error('native socket failed')));
  });
  let seq = 0;
  const pending = new Map<string, { resolve: (v: any) => void; reject: (e: Error) => void }>();
  const events: any[] = [];
  ws.addEventListener('message', (ev: MessageEvent) => {
    const f = JSON.parse(String(ev.data));
    if (f.op === 'event' && f.event?.type === 'sessionArchive.changed') events.push(f.event.data);
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
  ws.send(JSON.stringify({ op: 'subscribe', topics: ['sessionArchive.changed'] }));
  return { call, events, close: () => ws.close() };
}

async function openApp(page: Page) {
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto(hub.appUrl, { waitUntil: 'load' });
  const feed = page.getByTestId('sidebar-feed');
  await expect(feed.getByText('keep-me')).toBeVisible({ timeout: 60_000 });
  return feed;
}

/** Right-click a row until its menu offers `item` — the archive is read once
 *  the session is up, and only then can a row be archived. */
async function menuItem(page: Page, row: ReturnType<Page['getByText']>, item: string) {
  await expect(async () => {
    // An open menu sits under the pointer and would swallow the next click.
    // Its own capture-phase Escape handler closes it without the key ever
    // reaching the composer (where Escape would interrupt the agent).
    if (await page.getByRole('menu').count()) await page.keyboard.press('Escape');
    await row.click({ button: 'right' });
    await expect(page.getByRole('menuitem', { name: item })).toBeVisible({ timeout: 1_000 });
  }).toPass({ timeout: 20_000 });
  await page.getByRole('menuitem', { name: item }).click();
}

// What archiving must never send. (`claude.gate {on:true}` is NOT here: every
// GUI pane sends it on attach to ADD an approval gate; it stops nothing.)
/** Evidence for a reviewer, never an assertion: Chromium occasionally refuses
 *  a capture of a busy page, and that must not fail the regression. */
async function capture(page: Page, name: string) {
  await page.screenshot({ path: test.info().outputPath(name) }).catch(() => {});
}

const LIFECYCLE = ['claude.signal', 'agents.close', 'sessions.delete'];

test('a native archive hides the web row live, survives reload, and restores', async ({ page }) => {
  const native = await nativeClient(new URL(hub.url).port);
  try {
    const feed = await openApp(page);
    await expect(feed.getByText('archive-me')).toBeVisible();

    // Native archives ws2.
    const doc = await native.call('sessionArchive.set', { sessionId: 'ws2', archived: true });
    expect(doc.archived.ws2).toBeGreaterThan(0);
    await expect(feed.getByText('archive-me')).toHaveCount(0, { timeout: 10_000 });
    await expect(feed.getByText('keep-me')).toBeVisible();
    await capture(page, 'web-sidebar-after-native-archive.png');

    // Kept, not forgotten: the workspace is still in the shared layout.
    const layout = await native.call('layout.get', {});
    expect(JSON.stringify(layout.data)).toContain('agent-archive');

    // A reload re-reads the hub's archive.
    await page.reload({ waitUntil: 'load' });
    await expect(feed.getByText('keep-me')).toBeVisible({ timeout: 60_000 });
    await expect(feed.getByText('archive-me')).toHaveCount(0, { timeout: 10_000 });

    // The Archived view lists it; Restore brings it back for every client.
    await feed.getByTitle(/Sessions hidden from this list/).click();
    await expect(feed.getByText('Archived · 1')).toBeVisible();
    await expect(feed.getByText('archive-me')).toBeVisible();
    await capture(page, 'web-sidebar-archived-view.png');
    await menuItem(page, feed.getByText('archive-me'), 'Restore');
    await expect(feed.getByText('keep-me')).toBeVisible();
    await expect(feed.getByText('archive-me')).toBeVisible({ timeout: 10_000 });
    expect((await native.call('sessionArchive.get', {})).archived).toEqual({});

    for (const method of LIFECYCLE) expect(hub.callsTo(method), method).toEqual([]);
  } finally {
    native.close();
  }
});

test('a web archive reaches the native listener and never stops the agent', async ({ page }) => {
  const native = await nativeClient(new URL(hub.url).port);
  try {
    const feed = await openApp(page);
    await feed.getByText('keep-me').click();
    const composer = page.locator('textarea:visible').first();
    await composer.fill('KEEP_ARCHIVED_CONVERSATION_DRAFT', { timeout: 15_000 });
    await menuItem(page, feed.getByText('keep-me'), 'Archive');
    await expect(feed.getByText('keep-me')).toHaveCount(0, { timeout: 10_000 });
    await expect(composer).toHaveValue('KEEP_ARCHIVED_CONVERSATION_DRAFT');

    await expect.poll(() => native.events.some((d) => d?.archived?.ws1 > 0)).toBe(true);
    expect((await native.call('sessionArchive.get', {})).archived.ws1).toBeGreaterThan(0);
    for (const method of LIFECYCLE) expect(hub.callsTo(method), method).toEqual([]);
  } finally {
    native.close();
  }
});
