import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let base: string;
test.beforeAll(async () => {
  const port = await freePort();
  base = `http://127.0.0.1:${port}/routing-harness.html`;
  vite = spawn(
    process.execPath,
    [
      '--input-type=module',
      '-e',
      `import {createServer} from 'vite'; const server=await createServer({cacheDir:'../../test-results/routing-vite-cache',server:{host:'127.0.0.1',port:${port},strictPort:true,fs:{allow:['../../../../']}}});await server.listen();`,
    ],
    { cwd: path.resolve(__dirname, '../../src/renderer'), stdio: 'pipe' },
  );
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(base)).ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('isolated routing fixture did not start');
});
test.afterAll(() => vite?.kill());
for (const width of [360, 1280])
  for (const theme of ['light', 'dracula']) {
    test(`routing ${width}px ${theme} applies, resets and previews`, async ({ page }, info) => {
      await page.setViewportSize({ width, height: 900 });
      const errors: string[] = [];
      page.on('pageerror', (e) => errors.push(e.message));
      await page.goto(`${base}?theme=${theme}`);
      await page.getByLabel('scout capability', { exact: true }).selectOption('cheap');
      await page.getByRole('button', { name: 'Validate', exact: true }).click();
      await page.getByRole('button', { name: 'Apply', exact: true }).click();
      await expect(page.getByRole('status')).toHaveText('Applied to the live routing service.');
      await page.getByText('Try this route', { exact: true }).click();
      await page.getByRole('button', { name: 'Preview route', exact: true }).click();
      await expect(page.getByLabel('Route preview')).toContainText('gpt-5.6-luna');
      await page.getByText('Advanced policy', { exact: true }).click();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      expect(await page.locator('main').evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(
        true,
      );
      await page.locator('main').evaluate((el) => {
        el.scrollTop = 0;
      });
      await page.screenshot({
        path: info.outputPath(`routing-${width}-${theme}.png`),
        fullPage: false,
      });
      await page.getByRole('button', { name: 'Reset managed preferences', exact: true }).click();
      await expect(page.getByLabel('scout capability', { exact: true })).toHaveValue('balanced');
      expect(errors).toEqual([]);
    });
  }
