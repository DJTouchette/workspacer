/** Fresh generated-default profile, production App and first-use front doors.
 * Every host call is isolated at electronAPI; no live providers or profile files. */
import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let base: string;
test.beforeAll(async ({ browser }) => {
  console.info('First-use Chromium:', browser.version());
  const port = await freePort();
  base = `http://127.0.0.1:${port}/first-use-harness.html`;
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
  throw new Error('First-use harness failed to start');
});
test.afterAll(() => vite?.kill());
test.beforeEach(async ({ page }) => {
  page.on('pageerror', (error) => console.error('Browser error:', error.stack));
  await page.route('**/*', (route) => {
    const url = new URL(route.request().url());
    return url.origin === new URL(base).origin || ['data:', 'blob:'].includes(url.protocol)
      ? route.continue()
      : route.abort();
  });
});

const dialog = (page: any) => page.getByRole('dialog', { name: 'Dispatch agent' });
const launch = (page: any) =>
  dialog(page).getByRole('button', { name: 'Dispatch agent', exact: true });
const calls = (page: any) => page.evaluate(() => (window as any).firstUse.calls);
async function openFirstTask(page: any) {
  const welcome = page.getByRole('dialog', { name: 'Welcome' });
  await expect(welcome).toBeVisible();
  await expect(page.getByRole('button', { name: 'Start your first task' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByLabel('What should this agent do?')).toBeFocused();
}
async function assertNoAgentWrites(page: any) {
  const all = await calls(page);
  for (const call of all.filter((c: any) => ['layoutSet', 'saveSession'].includes(c.method))) {
    expect((call.args[0].agents ?? []).filter((a: any) => !a.global)).toEqual([]);
  }
  expect(
    all.filter(
      (c: any) =>
        c.method === 'saveConfig' && (c.args[0].agents || c.args[0].claude || c.args[0].projects),
    ),
  ).toEqual([]);
  expect(all.filter((c: any) => c.method === 'setActiveSession' && c.args[0])).toEqual([]);
  expect(await page.evaluate(() => (window as any).firstUse.snapshots())).toEqual({});
}

for (const theme of ['light', 'dracula']) {
  for (const width of [360, 1280]) {
    for (const provider of ['claude', 'codex']) {
      test(`first task recovery ${provider} ${theme} ${width}`, async ({ page }) => {
        const errors: string[] = [];
        page.on('pageerror', (error) => errors.push(error.message));
        await page.setViewportSize({ width, height: 900 });
        await page.goto(`${base}?theme=${theme}`);
        await expect(page.getByRole('dialog', { name: 'Welcome' })).toBeVisible();
        await expect(
          page.getByText('New: the tmux-style command layer', { exact: true }),
        ).toHaveCount(0);
        await page.screenshot({
          animations: 'disabled',
          path: test.info().outputPath('welcome.png'),
        });
        await openFirstTask(page);
        await expect(launch(page)).toBeDisabled();
        await expect(page.getByLabel(/Allow an empty session/)).toHaveCount(0);
        const task = `Explain this project ${provider}`;
        await page.getByLabel('What should this agent do?').fill(task);
        if (provider === 'codex') {
          await dialog(page).getByRole('button', { name: 'Codex', exact: true }).focus();
          await page.keyboard.press('Enter');
        }
        if (provider === 'claude') {
          await dialog(page)
            .getByRole('button', { name: /advanced/i })
            .click();
          await dialog(page).getByRole('button', { name: 'terminal', exact: true }).click();
        }
        await page.getByLabel('Working directory').fill('/fixture/project');
        await expect(dialog(page)).toContainText('Authentication has not been checked');
        await launch(page).focus();
        await page.keyboard.press('Enter');
        await expect(page.getByRole('alert')).toContainText('could not start');
        await expect(page.getByRole('alert')).not.toContainText('DO-NOT-DISPLAY');
        await expect(page.getByRole('alert')).toBeFocused();
        await expect(page.getByLabel('What should this agent do?')).toHaveValue(task);
        await expect(page.getByLabel('Working directory')).toHaveValue('/fixture/project');
        await assertNoAgentWrites(page);
        await dialog(page).getByRole('button', { name: 'Retry dispatch' }).focus();
        await page.screenshot({
          animations: 'disabled',
          path: test.info().outputPath('retry.png'),
        });
        const bounds = await dialog(page)
          .getByRole('button', { name: 'Retry dispatch' })
          .boundingBox();
        expect(bounds!.x).toBeGreaterThanOrEqual(0);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
        expect(
          await dialog(page).evaluate((el: HTMLElement) => el.scrollWidth <= el.clientWidth),
        ).toBe(true);
        await page.evaluate(() => (window as any).firstUse.spawnMode('success'));
        await dialog(page).getByRole('button', { name: 'Retry dispatch' }).focus();
        await page.keyboard.press('Enter');
        await expect(dialog(page)).toHaveCount(0);
        await expect
          .poll(() =>
            page.evaluate(
              () => (window as any).firstUse.layout()?.agents.filter((a: any) => !a.global).length,
            ),
          )
          .toBe(1);
        const all = await calls(page);
        const spawns = all.filter((c: any) => c.method === 'spawnClaude');
        expect(spawns).toHaveLength(2);
        expect(spawns[1].args[0]).toMatchObject({
          message: task,
          permissionMode: provider === 'claude' ? 'default' : 'ask',
          skipPermissions: false,
          transport: provider === 'claude' ? 'pty' : 'stream',
        });
        expect(all.filter((c: any) => c.method === 'claudeMessage')).toEqual([]);
        const sessions = (await page.evaluate(() =>
          Object.values((window as any).firstUse.snapshots()),
        )) as any[];
        expect(sessions).toHaveLength(1);
        if (provider === 'claude')
          await page.getByRole('button', { name: 'GUI', exact: true }).click();
        await expect(page.getByText(task, { exact: true }).first()).toBeVisible();
        expect(
          sessions[0].conversation.filter((m: any) => m.role === 'user' && m.content === task),
        ).toHaveLength(1);
        expect(errors).toEqual([]);
      });
    }
  }
}

test('missing provider, Codex-only, binary recheck and unknown host', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 900 });
  await page.goto(`${base}?providers=codex-only`);
  await expect(page.getByRole('dialog', { name: 'Welcome' })).toContainText(
    'Claude Code, which was not found',
  );
  await expect(page.getByRole('button', { name: 'Show me around' })).toHaveCount(0);
  await expect(page.getByRole('link', { name: 'Read the Workspacer docs' })).toBeVisible();
  await openFirstTask(page);
  await page.getByLabel('What should this agent do?').fill('Codex task');
  await expect(launch(page)).toBeDisabled();
  await expect(dialog(page).getByRole('status')).toContainText('Claude Code is not installed');
  await expect(page.getByLabel('Provider binary override')).toBeVisible();
  await dialog(page).getByRole('button', { name: 'Codex', exact: true }).click();
  await expect(launch(page)).toBeEnabled();
  await dialog(page)
    .getByRole('button', { name: /Claude Code/ })
    .click();
  await page.evaluate(() => (window as any).firstUse.providers('failed'));
  await dialog(page).getByRole('button', { name: 'Check again' }).click();
  await expect(dialog(page).getByRole('status')).toContainText('availability is unknown');
  await expect(launch(page)).toBeEnabled();
  await page.evaluate(() => (window as any).firstUse.providers('installed'));
  await dialog(page).getByRole('button', { name: 'Check again' }).click();
  await expect(dialog(page).getByRole('status')).toContainText('CLI found');
  expect(
    (await calls(page)).filter((c: any) => c.method === 'providerCheckAll' && c.args[0] === true)
      .length,
  ).toBeGreaterThanOrEqual(2);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Welcome' })).toBeVisible();
  await assertNoAgentWrites(page);
  expect((await calls(page)).filter((c: any) => c.method === 'spawnClaude')).toEqual([]);
});

for (const detection of ['unknown', 'failed', 'malformed']) {
  test(`detection ${detection} does not falsely block or promise readiness`, async ({ page }) => {
    await page.goto(`${base}?providers=${detection}&spawn=success`);
    await openFirstTask(page);
    await page.getByLabel('What should this agent do?').fill('Unknown availability task');
    await expect(dialog(page).getByRole('status')).toContainText('availability is unknown');
    await expect(launch(page)).toBeEnabled();
  });
}

test('malformed success leaves the direct front door retryable', async ({ page }) => {
  await page.goto(`${base}?spawn=malformed`);
  await page.getByRole('button', { name: "Got it — don't show again" }).click();
  await page.keyboard.press('Control+Shift+N');
  await page.getByLabel('What should this agent do?').fill('Direct task');
  await launch(page).click();
  await expect(page.getByRole('alert')).toContainText('could not start');
  await assertNoAgentWrites(page);
  await page.evaluate(() => (window as any).firstUse.spawnMode('success'));
  await page.getByRole('button', { name: 'Retry dispatch' }).click();
  await expect(dialog(page)).toHaveCount(0);
  expect(await page.evaluate(() => Object.keys((window as any).firstUse.snapshots()))).toHaveLength(
    1,
  );
});

test('Guide welcome launch retains its question and retries without dismissing welcome', async ({
  page,
}) => {
  await page.goto(base);
  await page.getByRole('button', { name: 'Show me around' }).click();
  await expect(page.getByRole('dialog', { name: 'Welcome' })).toBeVisible();
  await expect(page.getByRole('alert')).toContainText('Claude Code could not start');
  await assertNoAgentWrites(page);
  await page.evaluate(() => (window as any).firstUse.spawnMode('success'));
  await page.getByRole('button', { name: 'Show me around' }).click();
  await expect(page.getByRole('dialog', { name: 'Welcome' })).toHaveCount(0);
  const spawns = (await calls(page)).filter((c: any) => c.method === 'spawnClaude');
  expect(spawns).toHaveLength(2);
  expect(spawns[1].args[0]).toMatchObject({
    provider: 'claude',
    transport: 'stream',
    toolScope: 'triage',
  });
  expect(spawns[1].args[0].message).toContain('Give me a guided tour');
  expect((await calls(page)).filter((c: any) => c.method === 'claudeMessage')).toEqual([]);
});

test('Fleet Manager failure retains the ask and succeeds on retry', async ({ page }) => {
  await page.goto(base);
  await page.getByRole('button', { name: "Got it — don't show again" }).click();
  const ask = page.getByRole('textbox', { name: 'Ask the Fleet Manager' });
  await ask.fill('Coordinate my first task');
  await ask.press('Enter');
  await expect(page.getByRole('alert')).toContainText('could not start');
  await expect(ask).toHaveValue('Coordinate my first task');
  await expect(ask).toBeFocused();
  await assertNoAgentWrites(page);
  await page.evaluate(() => (window as any).firstUse.spawnMode('success'));
  await page.getByRole('button', { name: 'Retry Fleet Manager' }).click();
  const spawns = (await calls(page)).filter((c: any) => c.method === 'spawnClaude');
  expect(spawns).toHaveLength(2);
  expect(spawns[1].args[0]).toMatchObject({
    manager: true,
    fleetFullAccess: false,
    transport: 'stream',
  });
  expect(spawns[1].args[0].message).toContain('Coordinate my first task');
  expect(spawns[1].args[0].skipPermissions).not.toBe(true);
});

test('Codex-only clean profile starts its first task without a Claude promise', async ({
  page,
}) => {
  await page.goto(`${base}?providers=codex-only&spawn=success`);
  await expect(page.getByRole('button', { name: 'Show me around' })).toHaveCount(0);
  await openFirstTask(page);
  await page.getByLabel('What should this agent do?').fill('Only Codex is installed');
  await dialog(page).getByRole('button', { name: 'Codex', exact: true }).click();
  await launch(page).click();
  await expect(dialog(page)).toHaveCount(0);
  const spawns = (await calls(page)).filter((c: any) => c.method === 'spawnClaude');
  expect(spawns).toHaveLength(1);
  expect(spawns[0].args[0]).toMatchObject({
    provider: 'codex',
    message: 'Only Codex is installed',
    permissionMode: 'ask',
    skipPermissions: false,
  });
  expect(spawns[0].args[0].model ?? '').not.toContain('opus');
});

test('binary override recovers a confirmed missing provider and persists only the override', async ({
  page,
}) => {
  await page.goto(`${base}?providers=missing&spawn=success`);
  await openFirstTask(page);
  await page.getByLabel('What should this agent do?').fill('Use custom binary');
  await expect(launch(page)).toBeDisabled();
  const binary = page.getByLabel('Provider binary override');
  await binary.fill('/fixture/custom/claude');
  await binary.press('Tab');
  await expect(dialog(page).getByRole('status')).toContainText('CLI found');
  expect(await page.evaluate(() => (window as any).firstUse.config().agents.binaries.claude)).toBe(
    '/fixture/custom/claude',
  );
  await launch(page).click();
  await expect(dialog(page)).toHaveCount(0);
});

test('keyboard Tab stays in welcome and dispatch, including cancel and permissions', async ({
  page,
}) => {
  await page.setViewportSize({ width: 360, height: 900 });
  await page.goto(base);
  await expect(page.getByRole('button', { name: 'Start your first task' })).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(page.getByRole('button', { name: "Got it — don't show again" })).toBeFocused();
  await page.keyboard.press('Tab');
  await openFirstTask(page);
  await page.keyboard.press('Shift+Tab');
  await expect(dialog(page).getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByLabel('What should this agent do?')).toBeFocused();
  await page.keyboard.type('Keyboard-only task');
  let foundPermissions = false;
  for (let index = 0; index < 25; index++) {
    await page.keyboard.press('Tab');
    foundPermissions = await page.evaluate(
      () =>
        document.activeElement?.tagName === 'SELECT' &&
        !!document.activeElement?.textContent?.includes('Full access'),
    );
    if (foundPermissions) break;
  }
  expect(foundPermissions).toBe(true);
  expect(await page.evaluate(() => (document.activeElement as HTMLSelectElement).value)).toBe('');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Welcome' })).toBeVisible();
  await assertNoAgentWrites(page);
});

test('palette prompt dispatch rides spawn and a failure reopens its editable task', async ({
  page,
}) => {
  await page.goto(base);
  await page.getByRole('button', { name: "Got it — don't show again" }).click();
  await page.keyboard.press('Control+k');
  const query = page.getByPlaceholder('Search actions and apps…');
  await query.fill('Explain the first-use fixture');
  await query.press('Enter');
  await expect(page.getByLabel('What should this agent do?')).toHaveValue(
    'Explain the first-use fixture',
  );
  await assertNoAgentWrites(page);
  await page.evaluate(() => (window as any).firstUse.spawnMode('success'));
  await launch(page).click();
  await expect(dialog(page)).toHaveCount(0);
  const spawns = (await calls(page)).filter((c: any) => c.method === 'spawnClaude');
  expect(spawns).toHaveLength(2);
  expect(spawns.map((c: any) => c.args[0].message)).toEqual([
    'Explain the first-use fixture',
    'Explain the first-use fixture',
  ]);
  expect((await calls(page)).filter((c: any) => c.method === 'claudeMessage')).toEqual([]);
});

test('Guide pane front door retains a failed question and retries', async ({ page }) => {
  await page.goto(base);
  await page.getByRole('button', { name: "Got it — don't show again" }).click();
  await page.keyboard.press('Control+k');
  const query = page.getByPlaceholder('Search actions and apps…');
  await query.fill('Workspacer Guide');
  await query.press('Enter');
  const question = page.getByPlaceholder('Ask anything about Workspacer…');
  await question.fill('Where do results appear?');
  await question.press('Enter');
  await expect(page.getByRole('alert')).toContainText('could not start');
  await expect(question).toHaveValue('Where do results appear?');
  expect(await page.evaluate(() => (window as any).firstUse.snapshots())).toEqual({});
  await page.evaluate(() => (window as any).firstUse.spawnMode('success'));
  await question.press('Enter');
  expect((await calls(page)).filter((c: any) => c.method === 'spawnClaude')).toHaveLength(2);
});
