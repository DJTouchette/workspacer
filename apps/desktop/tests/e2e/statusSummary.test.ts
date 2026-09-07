import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let base: string;
test.beforeAll(async () => {
  const port = await freePort();
  base = `http://127.0.0.1:${port}/status-summary-harness.html`;
  // Private worktree build cache; never install into shared node_modules.
  vite = spawn(
    process.execPath,
    [
      '--input-type=module',
      '-e',
      `import {createServer} from 'vite'; const server=await createServer({cacheDir:'../../test-results/status-summary-vite-cache',server:{host:'127.0.0.1',port:${port},strictPort:true}});await server.listen();`,
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
  throw new Error('isolated settings fixture did not start');
});
test.afterAll(() => vite?.kill());
for (const width of [360, 768, 1280]) {
  test(`summary settings persist exact model and fit ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(e.message));
    await page.goto(base);
    const saved = page.getByTestId('saved-summary');
    await expect(saved).toContainText('"model":"haiku"');
    await page.getByRole('button', { name: 'Codex', exact: true }).click();
    await expect(saved).toContainText('"provider":"codex","model":null');
    await expect(page.getByText(/no-tools-unsupported/)).toBeVisible();
    const modelRow = page.getByText('Status summary model', { exact: true }).locator('..');
    await modelRow.getByRole('button').click();
    await modelRow.getByText('GPT-5 Nano', { exact: true }).click();
    await expect(saved).toContainText('"model":"gpt-5-nano"');
    await page
      .getByRole('checkbox', { name: 'Summarize agent status on demand' })
      .uncheck({ force: true });
    await expect(saved).toContainText('"enabled":false');
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: testInfo.outputPath(`settings-${width}.png`), fullPage: true });
    expect(errors).toEqual([]);
  });
}
