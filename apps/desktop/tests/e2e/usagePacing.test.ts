import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let url: string;
test.beforeAll(async () => {
  const port = await freePort();
  url = `http://127.0.0.1:${port}/usage-pacing-harness.html`;
  vite = spawn('npx', ['vite', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
    cwd: path.resolve(__dirname, '../../src/renderer'),
    stdio: 'ignore',
  });
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      /* starting */
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error('Usage harness did not start');
});
test.afterAll(() => {
  vite?.kill();
});
test('light/dark, narrow/wide consumption and keyboard detail', async ({ page }, info) => {
  for (const theme of ['light', 'dark'])
    for (const width of [360, 1200]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(`${url}?theme=${theme}`);
      await expect(page.getByText('ahead', { exact: true })).toBeVisible();
      await expect(page.getByText('on pace', { exact: true })).toBeVisible();
      await expect(page.getByTestId('usage-consumed').first()).toHaveCSS('width', '0px');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({
        path: info.outputPath(`usage-${theme}-${width}.png`),
        fullPage: true,
      });
      const identity = '/home/user/company/department/team/accounts/work';
      const card = page.getByRole('button', { name: `Show usage detail: claude · ${identity}` });
      await card.focus();
      await page.keyboard.press('Enter');
      await expect(page.getByRole('dialog').getByText(identity)).toBeVisible();
      await page.getByRole('button', { name: 'Close', exact: true }).click();
      await expect(page.getByRole('dialog')).toHaveCount(0);
    }
});
