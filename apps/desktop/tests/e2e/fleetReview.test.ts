/** Real Chromium, production FleetMessageCard/DiffViewer; evidence transport is a mock.
 * Real Git and production allocation/finish/cleanup are covered in fleetReviewStore.test.ts. */
import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let base: string;
test.beforeAll(async ({ browser }) => {
  console.info('Fleet review Chromium:', browser.version());
  const port = await freePort();
  base = `http://127.0.0.1:${port}/fleet-review-harness.html`;
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
  throw new Error('Fleet review harness failed to start');
});
test.afterAll(() => vite?.kill());
for (const width of [360, 1280])
  for (const theme of ['everforest', 'light']) {
    test(`inline diff at ${width}px in ${theme}, keyboard and revoke`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      await page.goto(`${base}?theme=${theme}`);
      const pages = page.context().pages().length;
      const review = page.getByRole('button', { name: 'Review changes' });
      await expect(review).toBeVisible();
      await expect(page.getByText(/Worker-reported/)).toBeVisible();
      await review.focus();
      await page.keyboard.press('Enter');
      const inline = page.getByTestId('fleet-inline-review');
      await expect(inline).toBeVisible();
      await expect(inline.getByText(/turn ended; writer may still be running/)).toBeVisible();
      const file = inline.getByRole('button', { name: 'M · src/changed.ts' });
      await file.focus();
      await expect(file).toHaveAttribute('aria-pressed', 'true');
      await expect(
        inline.getByText('export const value = "captured change";', { exact: true }),
      ).toBeVisible();
      expect(await inline.evaluate((el) => el.getBoundingClientRect().right <= innerWidth)).toBe(
        true,
      );
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      expect(page.context().pages()).toHaveLength(pages);
      expect(
        await page.evaluate(() => (window as unknown as { paneRequests: string[] }).paneRequests),
      ).toEqual([]);
      const calls = await page.evaluate(
        () => (window as unknown as { reviewCalls: unknown[] }).reviewCalls,
      );
      expect(calls).toEqual([
        {
          ownerSessionId: 'manager',
          workerSessionId: 'worker',
          evidenceId: '11111111-1111-4111-8111-111111111111',
        },
        {
          ownerSessionId: 'manager',
          workerSessionId: 'worker',
          evidenceId: '11111111-1111-4111-8111-111111111111',
          file: 'src/changed.ts',
        },
      ]);
      await page.screenshot({
        path: test.info().outputPath(`fleet-${width}-${theme}.png`),
        fullPage: true,
      });
      await inline.getByRole('button', { name: 'Forget review data' }).click();
      await expect(inline).toHaveCount(0);
      await expect(review).toBeDisabled();
      await page.getByRole('button', { name: 'After review' }).focus();
      await expect(page.getByRole('button', { name: 'After review' })).toBeFocused();
      expect(errors).toEqual([]);
    });
  }
