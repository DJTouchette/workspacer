import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import path from 'path';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let base: string;
test.beforeAll(async () => {
  const port = await freePort();
  base = `http://127.0.0.1:${port}/fleet-workflows-harness.html`;
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
    } catch {}
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('Workflow harness failed to start');
});
test.afterAll(() => vite?.kill());
for (const width of [360, 1280])
  for (const theme of ['light', 'dracula'])
    test(`workflow editor ${width}px ${theme}`, async ({ page }) => {
      const errors: string[] = [];
      page.on('pageerror', (e) => errors.push(e.message));
      await page.setViewportSize({ width, height: 900 });
      await page.goto(`${base}?theme=${theme}`);
      await expect(page.getByLabel('Default Fleet workflow')).toBeVisible();
      await page.getByLabel('Default Fleet workflow').selectOption('direct-implementation');
      await page
        .getByLabel('Fleet workflow for /project/fixture')
        .selectOption('scout-implement-review');
      await page.getByRole('button', { name: 'Clone', exact: true }).first().click();
      await expect(
        page.locator('strong').filter({ hasText: 'Scout → implement → independent review copy' }),
      ).toBeVisible();
      await page.getByRole('button', { name: 'Edit / inspect' }).last().click();
      await page.getByLabel('Workflow name').fill('Read and review');
      await page.getByRole('button', { name: 'Move down' }).nth(0).click();
      await expect(page.getByText('Step 1: implement', { exact: true })).toBeVisible();
      // Reordering is local until Save; buttons must never submit the form accidentally.
      expect(
        await page.evaluate(() =>
          (window as unknown as { workflowCalls: { op: string }[] }).workflowCalls.filter(
            (c) => c.op === 'update',
          ),
        ),
      ).toEqual([]);
      await page.getByLabel('Workflow name').scrollIntoViewIfNeeded();
      await page.screenshot({
        path: test.info().outputPath(`workflow-editor-${width}-${theme}.png`),
      });
      await page.getByRole('button', { name: 'Save definition' }).click();
      await expect(page.locator('strong').filter({ hasText: 'Read and review' })).toBeVisible();
      await page.getByText('Frozen policy and steps', { exact: true }).click();
      await expect(page.getByText('No unresolved material risk')).toBeVisible();
      await expect(page.getByText('review · planned', { exact: false })).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      expect(errors).toEqual([]);
      await page.screenshot({
        path: test.info().outputPath(`workflow-${width}-${theme}.png`),
        fullPage: false,
      });
    });
test('conflicts preserve draft and unavailable hosts say so', async ({ page }) => {
  await page.goto(`${base}?conflict`);
  await page.getByRole('button', { name: 'Create workflow' }).click();
  await page.getByLabel('Workflow name').fill('Keep my draft');
  await page.getByRole('button', { name: 'Save definition' }).click();
  await expect(page.getByRole('alert')).toContainText('draft is preserved');
  await expect(page.getByLabel('Workflow name')).toHaveValue('Keep my draft');
  await page.goto(`${base}?unavailable`);
  await expect(page.getByText(/Headless and older peers are unavailable/).first()).toBeVisible();
  await expect(page.getByRole('button', { name: 'Create workflow' })).toHaveCount(0);
});
