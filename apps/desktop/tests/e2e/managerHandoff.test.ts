import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import path from 'path';
import fs from 'fs';
import os from 'os';
import { once } from 'events';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let base: string;
let cache: string;
test.beforeAll(async () => {
  const port = await freePort();
  base = `http://127.0.0.1:${port}/fleet-workflow-harness.html`;
  cache = fs.mkdtempSync(path.join(os.tmpdir(), 'handoff-vite-cache-'));
  const options = { cacheDir: cache, server: { host: '127.0.0.1', port, strictPort: true } };
  vite = spawn(
    process.execPath,
    [
      '--input-type=module',
      '-e',
      `import { createServer } from 'vite'; const server = await createServer(${JSON.stringify(options)}); await server.listen();`,
    ],
    { cwd: path.resolve(__dirname, '../../src/renderer'), stdio: 'ignore' },
  );
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(base)).ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('Isolated handoff renderer harness did not start');
});
test.afterAll(async () => {
  if (vite && vite.exitCode === null && vite.signalCode === null) {
    const ended = once(vite, 'exit');
    vite.kill();
    await ended;
  }
  if (cache) fs.rmSync(cache, { recursive: true, force: true });
});
async function open(page: import('@playwright/test').Page, mode: string, fontProbe = false) {
  await page.goto(`${base}?handoff=${mode}${fontProbe ? '&fontRetirementProbe=1' : ''}`);
  await page.waitForFunction(() => !!(window as any).fleetHarness);
  await page.evaluate(() => (window as any).fleetHarness.pilot());
  await expect(
    page
      .getByTitle(
        'Checkpoint and replace this Fleet Manager in the same pane — local desktop and local workers only',
      )
      .filter({ visible: true }),
  ).toBeVisible();
}
test('automatic same-pane replacement and renderer reload preserve workspace identity', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => {
    errors.push(e.stack ?? e.message);
    console.error('Handoff page error:', e.stack ?? e.message);
  });
  await open(page, 'happy', true);
  const button = page
    .getByTitle(
      'Checkpoint and replace this Fleet Manager in the same pane — local desktop and local workers only',
    )
    .filter({ visible: true });
  await button.click();
  await expect(page.getByText('Manager handoff complete').filter({ visible: true })).toBeVisible();
  // The harness publishes agentRecords in an effect after the status renders.
  // Wait for that snapshot while retaining the complete workspace assertion.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const h = (window as any).fleetHarness;
        const manager = h.agentRecords().find((a: any) => a.id === 'manager');
        return {
          id: manager.id,
          sid: manager.sessionId,
          pane: manager.tabs[0].panes[0].id,
          count: h.agentRecords().filter((a: any) => a.sessionId === 'fixture-successor').length,
          starts: h.calls.filter(
            (c: any) => c.method === 'replacement' && c.args[0].action === 'start',
          ).length,
        };
      }),
    )
    .toEqual({
      id: 'manager',
      sid: 'fixture-successor',
      pane: 'p-manager',
      count: 1,
      starts: 1,
    });
  await expect(
    page.getByText('Fresh manager context; pending decisions retained.').filter({ visible: true }),
  ).toBeVisible();
  await page.evaluate(async () => {
    (window as any).fleetHarness.releaseFontRelayouts();
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  });
  expect(await page.evaluate(() => (window as any).fleetHarness.calls.filter((c: any) => c.method === 'message'))).toEqual([]);
  await page.reload();
  await page.waitForFunction(() => !!(window as any).fleetHarness);
  await page.evaluate(() => (window as any).fleetHarness.pilot());
  await expect(page.getByText('Manager handoff complete').filter({ visible: true })).toBeVisible();
  expect(errors).toEqual([]);
  await page.screenshot({ path: test.info().outputPath('manager-handoff-complete.png') });
});
test('failed preparation retains old manager and shows retryable error', async ({ page }) => {
  await open(page, 'fail');
  await page
    .getByTitle(
      'Checkpoint and replace this Fleet Manager in the same pane — local desktop and local workers only',
    )
    .filter({ visible: true })
    .click();
  await expect(
    page.getByText('Handoff failed — old manager retained').filter({ visible: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as any).fleetHarness.agentRecords().find((a: any) => a.id === 'manager').sessionId,
    ),
  ).toBe('s-manager');
});
test('uncertain acknowledgement remains visible without replay until explicit risk-labelled retry', async ({
  page,
}) => {
  await open(page, 'ambiguous');
  await page
    .getByTitle(
      'Checkpoint and replace this Fleet Manager in the same pane — local desktop and local workers only',
    )
    .filter({ visible: true })
    .click();
  await expect(
    page.getByText('Manager handoff needs attention').filter({ visible: true }),
  ).toBeVisible();
  await page.getByText('Inspect handoff and delivery evidence').filter({ visible: true }).click();
  await page.getByText('kickoff: uncertain').filter({ visible: true }).click();
  await expect(
    page.getByText('Preserved kickoff with pending decisions').filter({ visible: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as any).fleetHarness.calls.filter(
          (c: any) => c.method === 'replacement' && c.args[0].action === 'resolve-delivery',
        ).length,
    ),
  ).toBe(0);
  await page.getByRole('button', { name: 'Send again — may duplicate work' }).click();
  await expect(page.getByText('Manager handoff complete').filter({ visible: true })).toBeVisible();
});
test('unavailable host is explicit and never opens ordinary handoff dialog', async ({ page }) => {
  await open(page, 'unavailable');
  await page
    .getByTitle(
      'Checkpoint and replace this Fleet Manager in the same pane — local desktop and local workers only',
    )
    .filter({ visible: true })
    .click();
  await expect(
    page
      .getByText(
        'Automatic manager replacement requires an owned local desktop manager and only local workers.',
      )
      .filter({ visible: true }),
  ).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('ordinary-agent handoff still opens the provider/settings dialog', async ({ page }) => {
  await page.goto(`${base}?handoff=happy`);
  await page.waitForFunction(() => !!(window as any).fleetHarness);
  await page.evaluate(() => (window as any).fleetHarness.pilot('worker'));
  await page
    .getByTitle(
      'Hand off this session to a new agent — pick provider, model, effort and permissions (summarized brief, new session)',
    )
    .filter({ visible: true })
    .click();
  await expect(page.getByRole('dialog', { name: 'Hand off session' })).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as any).fleetHarness.calls.filter(
          (c: any) => c.method === 'replacement' && c.args[0].action === 'start',
        ).length,
    ),
  ).toBe(0);
});
