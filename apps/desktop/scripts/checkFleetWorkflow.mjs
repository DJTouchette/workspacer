/** Browser regression with only fabricated agents. No Electron, daemon, or provider process. */
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import fs from 'node:fs/promises';
const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = path.join(desktop, 'src/renderer');
const require = createRequire(path.join(root, 'package.json'));
const { createServer } = await import(
  pathToFileURL(path.join(path.dirname(require.resolve('vite/package.json')), 'dist/node/index.js'))
    .href
);
const { default: react } = await import(
  pathToFileURL(require.resolve('@vitejs/plugin-react')).href
);
const { chromium } = require('playwright');
const output = path.resolve(desktop, '../../.workspacer/reports/fleet-workflow');
await fs.mkdir(output, { recursive: true });
const server = await createServer({
  root,
  configFile: false,
  plugins: [react()],
  cacheDir: path.join(output, 'vite-cache'),
  server: { host: '127.0.0.1', port: 0, strictPort: true },
});
await server.listen();
const address = server.httpServer.address();
const origin = `http://127.0.0.1:${address.port}`;
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM_PATH || '/usr/bin/chromium',
  headless: true,
  args: ['--no-sandbox'],
});
const failures = [];
const screenshots = [];
try {
  for (const width of [1200, 360])
    for (const theme of ['everforest', 'light']) {
      const page = await browser.newPage({ viewport: { width, height: 900 } });
      const errors = [];
      page.on('pageerror', (e) => errors.push(e.message));
      await page.goto(`${origin}/fleet-workflow-harness.html?theme=${theme}`);
      const card = (id) => page.locator(`[data-fleet-agent="${id}"]`);
      await card('manager').waitFor();
      const first = await page
        .locator('[data-fleet-agent]')
        .first()
        .getAttribute('data-fleet-agent');
      assert.equal(first, 'manager');
      await page.evaluate(() =>
        window.fleetHarness.activity('s-worker', {
          ambientState: 'waiting_approval',
          lastActivity: Date.now(),
        }),
      );
      assert.equal(
        await page.locator('[data-fleet-agent]').first().getAttribute('data-fleet-agent'),
        'manager',
      );
      await page.getByPlaceholder('Filter agents…').fill('no match');
      assert.deepEqual(
        await page
          .locator('[data-fleet-agent]')
          .evaluateAll((els) => els.map((e) => e.dataset.fleetAgent)),
        ['manager', 'manager-two'],
      );
      await page.getByPlaceholder('Filter agents…').fill('');
      const recent = page.getByRole('button', { name: 'Recent agents', exact: true });
      const historyBox = await recent.boundingBox();
      assert(historyBox.width >= 110 && historyBox.height >= 34, 'Recent agents must be roomy');
      await recent.click();
      await page.getByRole('dialog', { name: 'Recent agents' }).waitFor();
      await page.getByRole('button', { name: 'Close history' }).click();
      const shot = async (name) => {
        const file = path.join(output, `${width}-${theme}-${name}.png`);
        await page.waitForTimeout(300); // let the chat reveal animation settle
        await page.screenshot({ path: file });
        screenshots.push(file);
      };
      await shot('cards');
      await card('manager').getByText('Fleet Manager', { exact: true }).click();
      const chat = page.locator('[data-fleet-chat="s-manager"]');
      const composer = chat.getByRole('textbox');
      await composer.waitFor();
      const inlineNote = chat.frameLocator('iframe').getByRole('textbox', { name: 'Inline note' });
      await inlineNote.fill('Retained inside HTML');
      await chat.getByRole('button', { name: 'Prefill: Use notes' }).click();
      await page.waitForFunction(
        (el) => el.value === 'Follow up on these notes',
        await composer.elementHandle(),
      );
      await composer.fill('Retained manager draft');
      const managerNode = await composer.evaluate((el) => {
        el.dataset.continuity = 'original';
        return true;
      });
      assert(managerNode);
      await page
        .getByRole('navigation', { name: 'Fleet agents' })
        .getByRole('button', { name: /worker agent/ })
        .first()
        .click();
      const workerChat = page.locator('[data-fleet-chat="s-worker"]');
      await workerChat.getByRole('textbox').waitFor();
      assert.equal(await workerChat.getByRole('textbox').inputValue(), '');
      await workerChat.getByRole('textbox').fill('Hello worker');
      await workerChat.getByRole('textbox').press('Enter');
      await page.waitForFunction(() =>
        window.fleetHarness.calls.some((c) => c.method === 'message' && c.args[0] === 's-worker'),
      );
      await page
        .getByRole('navigation', { name: 'Fleet agents' })
        .getByRole('button', { name: 'Fleet Manager', exact: true })
        .click();
      assert.equal(await composer.inputValue(), 'Retained manager draft');
      assert.equal(await composer.getAttribute('data-continuity'), 'original');
      assert.equal(await inlineNote.inputValue(), 'Retained inside HTML');
      const chatBox = await chat.boundingBox();
      assert(chatBox.height >= 300 && chatBox.width >= 300, 'chat remains readable');
      assert(
        await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
        'no page overflow',
      );
      await shot('chat');
      await page.getByRole('button', { name: 'Back to fleet', exact: true }).click();
      await page.getByRole('button', { name: 'List', exact: true }).click();
      assert.equal(
        await page.locator('[data-fleet-agent]').first().getAttribute('data-fleet-agent'),
        'manager',
      );
      if (width > 650) await page.getByRole('button', { name: 'Active', exact: true }).click();
      assert(
        await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
        'list has no page overflow',
      );
      await shot('list');
      assert.equal(
        await page.locator('[data-fleet-agent]').first().getAttribute('data-fleet-agent'),
        'manager',
      );
      await card('manager').click();
      assert.equal(await composer.inputValue(), 'Retained manager draft');
      assert.equal(await inlineNote.inputValue(), 'Retained inside HTML');
      await page.evaluate(() => window.fleetHarness.connection(false));
      assert.equal(
        await page
          .locator('.fleet-chat-layout')
          .getByRole('button', { name: 'Terminate', exact: true })
          .isEnabled(),
        false,
      );
      await page.evaluate(() => window.fleetHarness.connection(true));
      await page
        .getByRole('navigation', { name: 'Fleet agents' })
        .getByRole('button', { name: /remote agent/ })
        .click();
      await page.evaluate(() => window.fleetHarness.failTerminate(true));
      const expanded = page.locator('.fleet-chat-layout');
      await expanded.getByRole('button', { name: 'Terminate', exact: true }).click();
      await expanded.getByRole('button', { name: 'Cancel', exact: true }).click();
      assert.equal(
        await page.evaluate(
          () => window.fleetHarness.calls.filter((c) => c.method === 'signal').length,
        ),
        0,
      );
      await expanded.getByRole('button', { name: 'Terminate', exact: true }).click();
      await expanded.getByRole('button', { name: 'Confirm terminate', exact: true }).click();
      await expanded.getByRole('alert').waitFor();
      await shot('termination-error');
      assert(await page.evaluate(() => window.fleetHarness.agents().includes('remote')));
      await page.evaluate(() => window.fleetHarness.failTerminate(false));
      await expanded.getByRole('button', { name: 'Confirm terminate', exact: true }).click();
      await page.waitForFunction(() => !window.fleetHarness.agents().includes('remote'));
      assert.deepEqual(
        await page.evaluate(
          () => window.fleetHarness.calls.filter((c) => c.method === 'signal').at(-1).args,
        ),
        ['s-remote', 'SIGTERM'],
      );
      assert.equal(
        await card('offline').getByRole('button', { name: 'Terminate', exact: true }).isEnabled(),
        false,
      );
      assert.deepEqual(errors, [], 'no browser runtime errors');
      console.log(
        `PASS ${width}px ${theme}: manager order, filter/list, chat switch/draft/send, history, remote cancel/error/success, offline`,
      );
      await page.close();
    }
} catch (e) {
  failures.push(e.stack ?? String(e));
  for (const context of browser.contexts())
    for (const page of context.pages())
      await page.screenshot({ path: path.join(output, 'failure.png') }).catch(() => {});
} finally {
  await browser.close();
  await server.close();
  await fs.writeFile(
    path.join(output, 'result.json'),
    JSON.stringify({ origin, screenshots, failures }, null, 2),
  );
}
if (failures.length) throw new Error(failures.join('\n'));
console.log(`Screenshots: ${output}`);
