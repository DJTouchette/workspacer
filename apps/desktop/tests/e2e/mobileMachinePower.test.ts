import { test, expect, type WebSocketRoute } from '@playwright/test';
import { startMobileHub, HOST_TOKEN, type MobileHub } from './fixtures/mobileHub';

let hub: MobileHub;
test.beforeAll(async () => {
  hub = await startMobileHub();
});
test.afterAll(async () => {
  await hub?.stop();
});

test('Stop pauses reconnect and Wake explicitly opens a new connection', async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  let connections = 0;
  let stops = 0;
  let activities = 0;
  let socket: WebSocketRoute;
  await page.routeWebSocket('**/bus?*', (route) => {
    socket = route;
    connections++;
    const server = route.connectToServer();
    route.onMessage((raw) => {
      const frame = JSON.parse(String(raw));
      if (frame.op === 'activity') activities++;
      if (frame.op === 'call' && frame.method === 'machine.power') {
        route.send(
          JSON.stringify({
            op: 'result',
            id: frame.id,
            result: { canStop: true, label: 'Test server', wake: 'http', idleMode: 'observe', idle: {quiescent:false, calmSeconds:0, dwellSeconds:900, blockers:[{kind:'session-working',detail:'Agent is working'}]} },
          }),
        );
      } else if (frame.op === 'call' && frame.method === 'machine.stop') {
        stops++;
        route.send(JSON.stringify({ op: 'result', id: frame.id, result: { accepted: true } }));
      } else server.send(raw);
    });
  });
  await page.goto(`${hub.url}/m?token=${HOST_TOKEN}`);
  await expect(page.locator('#machineStop')).toBeVisible();
  expect(activities).toBeGreaterThan(0);
  await expect(page.locator('#machineIdle')).toBeVisible();
  const initialConnections = connections;
  page.once('dialog', (dialog) => dialog.accept());
  await page.locator('#machineStop').click();
  await expect(page.getByRole('heading', { name: 'Server disconnected' })).toBeVisible();
  await expect(page.locator('#app')).toBeHidden();
  await page.screenshot({ path: testInfo.outputPath('machine-paused.png') });
  expect(stops).toBe(1);
  await page.evaluate(() => {
    window.dispatchEvent(new Event('online'));
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await page.waitForTimeout(1500);
  expect(connections).toBe(initialConnections);
  await page.getByRole('button', { name: 'Wake server' }).click();
  await expect(page.locator('#app')).toBeVisible();
  await expect.poll(() => connections).toBe(initialConnections + 1);
  // Automatic stop, or Stop from another device, must land on the same gate.
  socket!.close({ code: 4001, reason: 'machine stopping' });
  await expect(page.getByRole('button', { name: 'Wake server' })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('button', { name: 'Wake server' })).toBeVisible();
  expect(connections).toBe(initialConnections + 1);
});
