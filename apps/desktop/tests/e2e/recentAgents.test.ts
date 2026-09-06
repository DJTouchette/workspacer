/** Real Chromium and production RecentAgentsPane + bridged backend.
 * IPC history/evidence and bus snapshots are explicit fixtures; no live agents. */
import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let base: string;
test.beforeAll(async ({ browser }) => {
  console.info('Recent agents Chromium:', browser.version());
  const port = await freePort();
  base = `http://127.0.0.1:${port}/recent-agents-harness.html`;
  vite = spawn(
    process.execPath,
    [
      path.resolve(__dirname, '../../src/renderer/node_modules/vite/bin/vite.js'),
      '--host',
      '127.0.0.1',
      '--port',
      String(port),
      '--strictPort',
    ],
    { cwd: path.resolve(__dirname, '../../src/renderer'), stdio: 'ignore' },
  );
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(base)).ok) return;
    } catch {
      /* starting */
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('Recent agents harness failed to start');
});
test.afterAll(() => vite?.kill());

test.beforeEach(async ({ page }) => {
  await page.routeWebSocket('**/fixture-bus*', (socket) => {
    socket.send(JSON.stringify({ op: 'hello', scope: 'operator' }));
    socket.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (frame.op === 'call')
        socket.send(
          JSON.stringify({
            op: 'result',
            id: frame.id,
            result:
              frame.method === 'sessions.snapshot'
                ? {
                    sessionId: frame.params.sessionId,
                    status:
                      page.url().includes('mode=owner-ended') ||
                      (page.url().includes('mode=worker-ended') &&
                        frame.params.sessionId === 'worker-0')
                        ? 'ended'
                        : 'active',
                    cwd: '/project/alpha',
                  }
                : [],
          }),
        );
    });
  });
});
for (const width of [360, 1280])
  for (const theme of ['everforest', 'light']) {
    test(`production bridge pane ${width}px ${theme}: loops filters keyboard partial metrics`, async ({
      page,
    }) => {
      const errors: string[] = [];
      page.on('pageerror', (e) => errors.push(e.message));
      await page.setViewportSize({ width, height: 900 });
      await page.goto(`${base}?theme=${theme}`);
      await expect(page.getByText('Unclassified standalone task', { exact: true })).toBeVisible();
      await expect(page.getByText('Another manager task', { exact: true })).toHaveCount(0);
      const task = page.locator('summary').filter({ hasText: 'Ship the full parser' });
      await task.focus();
      await page.keyboard.press('Enter');
      await expect(page.getByText('scout', { exact: true })).toBeVisible();
      await expect(page.locator('details[open] .recent-attempt')).toHaveCount(18);
      await expect(page.getByText(/partial 9\/18 reported/).first()).toBeVisible();
      await expect(page.getByText(/stale \/ last observed/).first()).toBeVisible();
      await page.getByRole('button', { name: 'Open agent' }).first().click();
      expect(
        await page.evaluate(() => (window as unknown as { opened: unknown[] }).opened),
      ).toEqual([
        { sessionId: 'worker-0', cwd: '/project/alpha', title: 'scout', provider: 'codex' },
      ]);
      await page.getByRole('button', { name: 'Review changes' }).first().click();
      await expect(page.getByText('Review evidence evicted or revoked')).toBeVisible();
      expect(
        await page.evaluate(() => (window as unknown as { reviewCalls: unknown[] }).reviewCalls),
      ).toEqual([
        { ownerSessionId: 'manager', workerSessionId: 'worker-0', evidenceId: 'evidence' },
      ]);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({
        path: test.info().outputPath(`recent-${width}-${theme}.png`),
        fullPage: false,
      });
      await page.getByLabel('Project', { exact: true }).selectOption('/project/beta');
      await expect(task).toHaveCount(0);
      await page.getByLabel('Status', { exact: true }).selectOption('ended');
      await expect(page.getByText('No recorded dispatches match these filters.')).toBeVisible();
      await page.getByLabel('Status', { exact: true }).selectOption('all');
      await page.getByLabel('Manager', { exact: true }).selectOption('all');
      await expect(page.getByText('Another manager task', { exact: true })).toBeVisible();
      await page.locator('summary').filter({ hasText: 'Another manager task' }).click();
      await expect(page.getByRole('button', { name: 'Review changes' })).toBeDisabled();
      expect(errors).toEqual([]);
    });
  }
for (const mode of ['empty', 'unsupported', 'stopped'])
  test(`honest ${mode} state`, async ({ page }) => {
    await page.goto(`${base}?mode=${mode}`);
    await expect(
      page.getByText(
        mode === 'empty'
          ? 'No recorded dispatches match these filters.'
          : mode === 'unsupported'
            ? 'History unavailable'
            : 'No live local manager. Select all locally recorded managers to inspect retained history.',
        { exact: true },
      ),
    ).toBeVisible();
  });

test('a stopped owner cannot use a stale history response to read review evidence', async ({
  page,
}) => {
  await page.goto(`${base}?mode=owner-ended`);
  await page.locator('summary').filter({ hasText: 'Ship the full parser' }).click();
  await page.getByRole('button', { name: 'Review changes' }).first().click();
  await expect(page.getByText('The current owning manager is no longer available.')).toBeVisible();
  expect(
    await page.evaluate(() => (window as unknown as { reviewCalls: unknown[] }).reviewCalls),
  ).toEqual([]);
});
test('Open agent rechecks exact live state before attaching', async ({ page }) => {
  await page.goto(`${base}?mode=worker-ended`);
  await page.locator('summary').filter({ hasText: 'Ship the full parser' }).click();
  await page.getByRole('button', { name: 'Open agent' }).first().click();
  await expect(page.getByText('This agent is no longer available locally.')).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { opened: unknown[] }).opened)).toEqual(
    [],
  );
});
