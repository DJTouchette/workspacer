/** Production web publication over the actual Rust broker; no provider/model effects. */
import { test, expect } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import WebSocket from 'ws';
import { startAppHub, HOST_TOKEN, type AppHub } from './fixtures/appHub';

let hub: AppHub;
test.beforeAll(async () => {
  hub = await startAppHub();
});
test.afterAll(async () => {
  await hub?.stop();
});

test('web publication reaches Rust subscribers and view-token refusal cannot publish', async ({
  browser,
}) => {
  const topic = 'ui.publication-fixture';
  const observer = new WebSocket(`${hub.url.replace('http:', 'ws:')}/bus?token=${HOST_TOKEN}`);
  const frames: any[] = [];
  observer.on('message', (bytes) => frames.push(JSON.parse(String(bytes))));
  const context = await browser.newContext();
  try {
    // Publication needs the local application, not third-party font/network loads.
    await context.route('**/*', (route) => {
      const url = new URL(route.request().url());
      return url.origin === hub.url || ['data:', 'blob:'].includes(url.protocol)
        ? route.continue()
        : route.abort();
    });
    await expect.poll(() => frames.some((frame) => frame.op === 'hello')).toBe(true);
    observer.send(JSON.stringify({ op: 'subscribe', topics: [topic] }));
    await expect.poll(() => frames.some((frame) => frame.op === 'subscribed')).toBe(true);
    // Reproduce the legacy serialized RPC on this same real Rust broker; the
    // actual browser client below must use publish instead of swallowing it.
    observer.send(
      JSON.stringify({
        op: 'call',
        id: 'legacy-publish',
        method: '__publish',
        params: { type: topic, data: { legacyProbe: true } },
      }),
    );
    await expect
      .poll(() => frames.find((frame) => frame.id === 'legacy-publish')?.error)
      .toBe('no provider for __publish');

    const page = await context.newPage();
    await page.goto(hub.appUrl, { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(
      () => typeof window.electronAPI?.hubPublish === 'function',
      undefined,
      { timeout: 8000 },
    );
    await page.evaluate(() => window.electronAPI.getConfig());
    console.info('publication fixture: owner browser and observer ready');
    const marker = randomUUID();
    const event = {
      type: topic,
      source: 'workspacer.ui',
      data: { marker, text: 'exact 🦀', nested: [1, false] },
    };
    await page.evaluate((event) => window.electronAPI.hubPublish(event), event);
    await expect
      .poll(() => frames.find((frame) => frame.event?.data?.marker === marker)?.event?.data)
      .toEqual(event.data);
    const delivered = frames.find((frame) => frame.event?.data?.marker === marker).event;
    expect(delivered.type).toBe(topic);
    expect(delivered.source).toBe(event.source);
    expect(delivered.id).toBeTruthy();
    expect(delivered.time).toBeTruthy();
    expect(hub.callsTo('__publish')).toEqual([]);
    console.info('publication fixture: actual Rust delivered owner event');

    const viewToken = randomUUID();
    const tokenFile = path.join(hub.scratchDir, 'config/workspacer/tokens.json');
    const records = JSON.parse(fs.readFileSync(tokenFile, 'utf8'));
    records.push({ token: viewToken, scope: 'view', label: 'publication refusal fixture' });
    fs.writeFileSync(tokenFile, JSON.stringify(records));
    // /app itself is operator-only. Authorize only this document fetch so the
    // production client code can run with a view-token WebSocket; this header
    // is never installed globally or sent on the /bus handshake.
    expect((await context.request.get(`${hub.url}/app/?token=${viewToken}`)).status()).toBe(401);
    const readonly = await context.newPage();
    const viewFrames: any[] = [];
    readonly.on('websocket', (socket) =>
      socket.on('framereceived', (frame) => {
        try {
          viewFrames.push(JSON.parse(String(frame.payload)));
        } catch {
          /* not a bus JSON frame */
        }
      }),
    );
    await readonly.route(
      (url) => url.origin === hub.url && url.pathname === '/app/',
      (route) =>
        route.continue({
          headers: { ...route.request().headers(), Authorization: `Bearer ${HOST_TOKEN}` },
        }),
    );
    const response = await readonly.goto(`${hub.url}/app/?token=${viewToken}`, {
      waitUntil: 'domcontentloaded',
    });
    expect(response?.status()).toBe(200);
    console.info('publication fixture: view document loaded');
    readonly.on('pageerror', (error) => console.info('view page error:', error.message));
    await readonly.waitForFunction(
      () => typeof window.electronAPI?.hubPublish === 'function',
      undefined,
      { timeout: 8000 },
    );
    console.info('publication fixture: view backend installed');
    await readonly.evaluate(() => window.electronAPI.getConfig());
    await expect
      .poll(() => viewFrames.some((frame) => frame.op === 'hello' && frame.scope === 'view'))
      .toBe(true);
    console.info('publication fixture: view browser ready');
    const refused = randomUUID();
    await readonly.evaluate(
      ({ topic, refused }) =>
        window.electronAPI.hubPublish({ type: topic, data: { marker: refused } }),
      { topic, refused },
    );
    // Same-socket RPC completion proves the broker processed the publication.
    // The subsequent host marker drains the subscriber's ordered event stream;
    // an unauthorized publication cannot hide behind an arbitrary quiet sleep.
    await readonly.evaluate(() => window.electronAPI.getConfig());
    await expect
      .poll(() =>
        viewFrames.some(
          (frame) =>
            frame.op === 'error' && frame.error?.includes('not authorized: publishing events'),
        ),
      )
      .toBe(true);
    const barrier = randomUUID();
    observer.send(
      JSON.stringify({
        op: 'publish',
        event: { type: topic, source: 'fixture', data: { marker: barrier } },
      }),
    );
    await expect
      .poll(() => frames.some((frame) => frame.event?.data?.marker === barrier))
      .toBe(true);
    expect(frames.some((frame) => frame.event?.data?.marker === refused)).toBe(false);
  } finally {
    observer.terminate();
    await context.close();
  }
});
