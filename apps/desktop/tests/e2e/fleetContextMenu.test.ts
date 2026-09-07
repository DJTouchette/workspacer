/** Real Chromium, production FleetDeck/ClaudePane; evidence transport is a mock.
 * Backend calls are isolated at electronAPI; no live sessions are contacted. */
import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let base: string;
test.beforeAll(async ({ browser }) => {
  console.info('Fleet context menu Chromium:', browser.version());
  const port = await freePort();
  base = `http://127.0.0.1:${port}/fleet-workflow-harness.html`;
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
  throw new Error('Fleet workflow harness failed to start');
});
test.afterAll(() => vite?.kill());
test.beforeEach(async ({ page }) => {
  page.on('pageerror', (error) => console.error('Browser error:', error.message));
});

for (const width of [360, 1280]) {
  test(`overview menu, confirmation, routing and full-width chat at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto(base);
    await page.waitForFunction(() => !!(window as any).fleetHarness?.agents().includes('manager'));
    const card = (id: string) => page.locator(`[data-fleet-agent="${id}"]:visible`);
    const calls = () => page.evaluate(() => (window as any).fleetHarness.calls);
    const menu = page.getByRole('menu');
    const terminate = page.getByRole('menuitem', { name: 'Terminate', exact: true });
    await expect(card('manager')).toBeVisible();
    await card('manager').focus();
    await expect(page.getByRole('button', { name: 'Terminate', exact: true })).toHaveCount(0);
    // A real secondary-button click includes mousedown: it must not select the worker.
    await card('worker').click({ button: 'right', position: { x: 40, y: 20 } });
    await expect(terminate).toBeFocused();
    await expect(page.locator('.fleet-root')).toContainText(/Selected\s*Fleet Manager/);
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    await expect(card('manager')).toBeFocused();
    expect(await calls()).toEqual([]);
    // Touch alternative and explicit cancel, without ever opening chat.
    await card('worker').getByRole('button', { name: 'Actions for worker agent' }).click();
    await terminate.click();
    await expect(page.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    expect(await calls()).toEqual([]);
    await expect(page.locator('[data-fleet-chat="s-worker"]')).toHaveCount(0);
    // Native context-menu key and Shift+F10, menu arrows, Tab and outside dismissal.
    await card('worker').focus();
    await page.keyboard.press('Shift+F10');
    await expect(terminate).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(terminate).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(menu).toHaveCount(0);
    await page.keyboard.press('ContextMenu');
    await expect(menu).toBeVisible();
    await page.getByText('Fleet', { exact: true }).click();
    await expect(menu).toHaveCount(0);
    expect(await calls()).toEqual([]);
    // Manager stays first after activity changes and when switching to List.
    await page.evaluate(() =>
      (window as any).fleetHarness.activity('s-worker', { ambientState: 'waiting-input' }),
    );
    await expect(page.locator('[data-fleet-agent]:visible').first()).toHaveAttribute(
      'data-fleet-agent',
      'manager',
    );
    await page.getByRole('button', { name: 'List', exact: true }).click();
    await expect(page.locator('[data-fleet-row]').first()).toHaveAttribute(
      'data-fleet-row',
      'manager',
    );
    await card('manager').focus();
    await card('worker').click({ button: 'right', position: { x: 40, y: 20 } });
    await expect(page.locator('.fleet-root')).toContainText(/Selected\s*Fleet Manager/);
    await terminate.click();
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    expect(await calls()).toEqual([]);
    // Offline permission reason stays inside the menu.
    await card('offline').getByRole('button', { name: 'Actions for offline agent' }).click();
    await expect(terminate).toBeDisabled();
    await expect(menu).toContainText('Hub is offline');
    await page.keyboard.press('Escape');
    await page.evaluate(() => (window as any).fleetHarness.connection(false));
    await card('worker').getByRole('button', { name: 'Actions for worker agent' }).click();
    await expect(terminate).toBeDisabled();
    await expect(menu).toContainText('Hub connection is offline');
    await page.keyboard.press('Escape');
    await page.evaluate(() => (window as any).fleetHarness.connection(true));
    // Rejected remote termination retains its row; retry uses canonical remote routing.
    await page.evaluate(() => (window as any).fleetHarness.failTerminate(true));
    await card('remote').getByRole('button', { name: 'Actions for remote agent' }).click();
    await terminate.click();
    await page.getByRole('button', { name: 'Confirm terminate', exact: true }).click();
    await expect(page.getByRole('alert')).toContainText('Fixture authority denied termination');
    await expect(card('remote')).toBeVisible();
    await page.evaluate(() => (window as any).fleetHarness.failTerminate(false));
    await page.getByRole('button', { name: 'Confirm terminate', exact: true }).click();
    await expect(card('remote')).toHaveCount(0);
    await expect(menu).toHaveCount(0);
    expect(await calls()).toEqual([
      { method: 'signal', args: ['s-remote', 'SIGTERM'] },
      { method: 'signal', args: ['s-remote', 'SIGTERM'] },
    ]);
    await card('worker').getByRole('button', { name: 'Actions for worker agent' }).click();
    await terminate.click();
    const bounds = await menu.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
    await page.getByRole('button', { name: 'Confirm terminate', exact: true }).click();
    await expect(card('worker')).toHaveCount(0);
    expect((await calls()).at(-1)).toEqual({ method: 'close', args: ['s-worker'] });
    // Existing entry point and retained production chat: no sidebar or header actions.
    await card('manager').click({ position: { x: 40, y: 20 } });
    const chat = page.locator('[data-fleet-chat="s-manager"]');
    await expect(chat).toBeVisible();
    await expect(page.getByRole('navigation', { name: 'Fleet agents' })).toHaveCount(0);
    const header = page.getByRole('button', { name: 'Back to fleet', exact: true }).locator('..');
    await expect(header).toContainText('Fleet Manager');
    await expect(header.getByRole('button')).toHaveCount(1);
    const chatBounds = await chat.boundingBox();
    expect(chatBounds!.width).toBe(width);
    const draft = chat.locator('textarea:visible').first();
    await draft.fill('Retain this unsent draft');
    const note = chat.frameLocator('iframe').getByRole('textbox', { name: 'Inline note' });
    await note.fill('Retain HTML document state');
    await page.getByRole('button', { name: 'Back to fleet', exact: true }).click();
    await expect(card('manager')).toBeFocused();
    await card('manager').press('Enter');
    await expect(draft).toHaveValue('Retain this unsent draft');
    await expect(note).toHaveValue('Retain HTML document state');
    await page.screenshot({ path: test.info().outputPath(`fleet-chat-${width}.png`) });
    await page.getByRole('button', { name: 'Back to fleet', exact: true }).focus();
    await page.keyboard.press('Escape');
    await expect(card('manager')).toBeVisible();
    expect(errors).toEqual([]);
  });
}

test('card send reaches the provider boundary and appears in its own card', async ({ page }) => {
  await page.goto(base);
  await page.waitForFunction(() => !!(window as any).fleetHarness?.agents().includes('manager'));
  const worker = page.locator('[data-fleet-agent="worker"]');
  await worker.getByRole('textbox').fill('Card message visibility regression');
  await worker.getByRole('button', { name: 'Send', exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).fleetHarness.calls))
    .toEqual([{ method: 'message', args: ['s-worker', 'Card message visibility regression'] }]);
  await expect(
    worker.getByText('Card message visibility regression', { exact: true }),
  ).toBeVisible();
});

for (const width of [360, 1280]) {
  test(`card pending, echo, failure and session isolation at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(base);
    await page.waitForFunction(() => !!(window as any).fleetHarness?.agents().includes('manager'));
    const card = (id: string) => page.locator(`[data-fleet-agent="${id}"]:visible`);
    const worker = card('worker');
    const text = 'Queued from the worker card';
    await page.evaluate(() => (window as any).fleetHarness.sendMode('defer'));
    await worker.getByRole('textbox').fill(text);
    await worker.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(worker.locator('p').filter({ hasText: text })).toBeVisible();
    await expect(worker.getByTestId('fleet-message-status')).toHaveText('You · queued');
    await expect(worker.getByRole('button', { name: 'Sending…', exact: true })).toBeDisabled();
    await expect(card('manager').getByText(text, { exact: true })).toHaveCount(0);
    // Pending state belongs to the owning chat and survives Back before acknowledgement.
    await worker.click({ position: { x: 40, y: 20 } });
    const chat = page.locator('[data-fleet-chat="s-worker"]');
    await expect(chat.getByText(text, { exact: true })).toBeVisible();
    const fullDraft = chat.locator('textarea:visible').first();
    await fullDraft.fill('A separate expanded-chat draft');
    await page.getByRole('button', { name: 'Back to fleet', exact: true }).click();
    await expect(worker.locator('p').filter({ hasText: text })).toBeVisible();
    // Preserve edits made while the request is outstanding.
    await worker.getByRole('textbox').fill('Next card draft');
    await page.evaluate(() => (window as any).fleetHarness.settleSend());
    await expect(worker.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
    await expect(worker.getByRole('textbox')).toHaveValue('Next card draft');
    await page.evaluate((message) => (window as any).fleetHarness.echo('s-worker', message), text);
    await expect(worker.getByTestId('fleet-message-status')).toHaveText('You');
    await page.evaluate(() => (window as any).fleetHarness.repeatSnapshot('s-worker'));
    await expect(worker.locator('p').filter({ hasText: text })).toHaveCount(1);
    await worker.click({ position: { x: 40, y: 20 } });
    await expect(chat.getByText(text, { exact: true })).toHaveCount(1);
    await expect(fullDraft).toHaveValue('A separate expanded-chat draft');
    await page.getByRole('button', { name: 'Back to fleet', exact: true }).click();
    // Provider refusal and transport failure retract the provisional turn and keep the draft.
    for (const mode of ['reject', 'throw']) {
      await page.evaluate((value) => (window as any).fleetHarness.sendMode(value), mode);
      await worker.getByRole('textbox').fill(`Retry after ${mode}`);
      await worker.getByRole('textbox').press('Enter');
      await expect(worker.getByRole('alert')).toContainText(
        mode === 'reject' ? 'not accepting input' : 'transport unavailable',
      );
      await expect(worker.getByRole('textbox')).toHaveValue(`Retry after ${mode}`);
      await expect(worker.locator('p').filter({ hasText: `Retry after ${mode}` })).toHaveCount(0);
      await worker.click({ position: { x: 40, y: 20 } });
      await expect(fullDraft).toHaveValue('A separate expanded-chat draft');
      await expect(chat.getByText(`Retry after ${mode}`, { exact: true })).toHaveCount(0);
      await page.getByRole('button', { name: 'Back to fleet', exact: true }).click();
      await expect(worker.getByRole('textbox')).toHaveValue(`Retry after ${mode}`);
    }
    await page.evaluate(() => (window as any).fleetHarness.sendMode('accept'));
    await worker.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(worker.getByRole('alert')).toHaveCount(0);
    await expect(worker.getByRole('textbox')).toHaveValue('');
    await expect(worker.getByText('Retry after throw', { exact: true })).toBeVisible();
    // A second card send has its own queue and cannot borrow the worker's draft or identity.
    await card('manager').getByRole('textbox').fill('Only for the manager');
    await card('manager').getByRole('textbox').press('Enter');
    await expect(card('manager').getByText('Only for the manager', { exact: true })).toBeVisible();
    await expect(worker.getByText('Only for the manager', { exact: true })).toHaveCount(0);
    const calls = await page.evaluate(() => (window as any).fleetHarness.calls);
    expect(calls).toEqual([
      { method: 'message', args: ['s-worker', text] },
      { method: 'message', args: ['s-worker', 'Retry after reject'] },
      { method: 'message', args: ['s-worker', 'Retry after throw'] },
      { method: 'message', args: ['s-worker', 'Retry after throw'] },
      { method: 'message', args: ['s-manager', 'Only for the manager'] },
    ]);
  });
}

test.describe('touch overview actions', () => {
  test.use({ hasTouch: true, viewport: { width: 360, height: 900 } });
  test('ellipsis opens confirmation and cancel preserves the card', async ({ page }) => {
    await page.goto(base);
    await page.waitForFunction(() => !!(window as any).fleetHarness?.agents().includes('manager'));
    const worker = page.locator('[data-fleet-agent="worker"]');
    await worker.getByRole('button', { name: 'Actions for worker agent' }).tap();
    await page.getByRole('menuitem', { name: 'Terminate', exact: true }).tap();
    await page.getByRole('button', { name: 'Cancel', exact: true }).tap();
    await expect(page.getByRole('menu')).toHaveCount(0);
    await expect(worker).toBeVisible();
    expect(await page.evaluate(() => (window as any).fleetHarness.calls)).toEqual([]);
  });
});

test('card send restores a missing owning chat without opening it', async ({ page }) => {
  await page.goto(`${base}?missingChat=worker`);
  await page.waitForFunction(() => !!(window as any).fleetHarness?.agents().includes('worker'));
  const worker = page.locator('[data-fleet-agent="worker"]');
  await worker.getByRole('textbox').fill('Send through the restored owner');
  await worker.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(worker.getByText('Send through the restored owner', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back to fleet' })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).fleetHarness.calls)).toEqual([
    { method: 'message', args: ['s-worker', 'Send through the restored owner'] },
  ]);
});
