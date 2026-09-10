/** Fresh generated-default profile, production App and first-use front doors.
 * Every host call is isolated at electronAPI; no live providers or profile files. */
import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';
import { once } from 'events';
import { freePort } from './fixtures/scratchState';
let vite: ChildProcess;
let base: string;
let cache: string;
test.beforeAll(async ({ browser }) => {
  console.info('First-use Chromium:', browser.version());
  const port = await freePort();
  base = `http://127.0.0.1:${port}/first-use-harness.html`;
  cache = fs.mkdtempSync(path.join(os.tmpdir(), 'isolated-renderer-cache-'));
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
    } catch {
      /* starting */
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('First-use harness failed to start');
});
test.afterAll(async () => {
  if (vite && vite.exitCode === null && vite.signalCode === null) {
    const ended = once(vite, 'exit');
    vite.kill();
    await ended;
  }
  if (cache) fs.rmSync(cache, { recursive: true, force: true });
});
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

for (const uiMode of ['focus', 'fleet']) {
  for (const viewLevel of ['piloting', 'fleet']) {
    test(`Overview navigation after restoring ${uiMode}/${viewLevel}`, async ({ page }) => {
      test.setTimeout(60000);
      await page.setViewportSize({ width: 1280, height: 900 });
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      await page.goto(`${base}?spawn=success&runtime=ready`);
      await page.getByRole('button', { name: "Got it — don't show again" }).click();
      const palette = async (query: string) => {
        await page.keyboard.press('Control+k');
        const input = page.getByPlaceholder('Search actions and apps…');
        await input.fill(query);
        await page
          .locator('[data-palette-row]')
          .filter({
            has: page.getByText(query, { exact: true }),
          })
          .click();
        await expect(input).toHaveCount(0);
      };
      // A real (fixture-backed) manager ensures the restored Fleet overlay mounts.
      await page.getByLabel('Ask the Fleet Manager').fill('Navigation fixture');
      await page.getByRole('button', { name: 'Ask Fleet Manager', exact: true }).click();
      await expect
        .poll(() => page.evaluate(() => Object.keys((window as any).firstUse.snapshots()).length))
        .toBe(1);
      await palette('Recent agents');
      await expect(page.getByRole('region', { name: 'Recent agents' })).toBeVisible();
      await expect
        .poll(() =>
          page.evaluate(() => {
            const layout = (window as any).firstUse.layout();
            const ws = layout?.agents.find((a: any) => a.global);
            return ws?.tabs.find((t: any) => t.id === ws.activeTabId)?.panes[0].type;
          }),
        )
        .toBe('recentagents');
      await page.evaluate(
        ({ uiMode, viewLevel }) => {
          const fixture = (window as any).firstUse;
          fixture.config().ui.mode = uiMode;
          fixture.config().panes.viewLevel = viewLevel;
          fixture.restart();
        },
        { uiMode, viewLevel },
      );
      await page.reload();
      const deck = page.locator('.fleet-root');
      if (uiMode === 'fleet' && viewLevel === 'fleet') {
        await expect(deck.getByRole('heading', { name: 'Runbook timeline' })).toBeVisible();
        await deck
          .getByRole('button', { name: 'Overview Usage, fleet status, projects and plugins' })
          .click();
      } else {
        await expect(deck).toHaveCount(0);
        await page.getByRole('button', { name: 'Overview', exact: true }).click();
      }
      const dashboard = page.getByText('Workspace', { exact: true });
      await expect(dashboard).toBeInViewport();
      await expect(page.getByText('All time', { exact: true })).toBeVisible();
      await expect(deck).toHaveCount(0);
      expect(await page.evaluate(() => (window as any).firstUse.config().ui.mode)).toBe(uiMode);
      expect(await page.evaluate(() => (window as any).firstUse.config().panes.viewLevel)).toBe(
        'piloting',
      );

      // Ordinary palette works with the command layer disabled by default.
      await palette('Recent agents');
      await palette('Open Overview');
      await expect(dashboard).toBeInViewport();
      // Brand/home and collapsed rail share the same route after history was selected.
      await palette('Recent agents');
      await page.getByTitle('Overview — cross-agent dashboards & plugin panes').click();
      await expect(dashboard).toBeInViewport();
      await palette('Recent agents');
      await page.keyboard.press('Control+b');
      await page.getByRole('button', { name: 'Overview', exact: true }).click();
      await expect(dashboard).toBeInViewport();
      // Existing command-layer key uses the same action (prefix 0).
      await palette('Enable Command Layer (tmux-style)');
      await palette('Recent agents');
      // resolveLeader substitutes an Alt tap for ctrl+space on this Linux fixture.
      await page.keyboard.press('Alt');
      await page.keyboard.press('0');
      await expect(dashboard).toBeInViewport();
      await expect
        .poll(() =>
          page.evaluate(() => {
            const layout = (window as any).firstUse.layout();
            const ws = layout?.agents.find((a: any) => a.global);
            return ws?.tabs.find((t: any) => t.id === ws.activeTabId)?.panes[0].type;
          }),
        )
        .toBe('overview');
      const types = await page.evaluate(() =>
        (window as any).firstUse
          .layout()
          .agents.find((a: any) => a.global)
          .tabs.flatMap((t: any) => t.panes.map((p: any) => p.type)),
      );
      expect(types.filter((type: string) => type === 'overview')).toHaveLength(1);
      expect(types.filter((type: string) => type === 'recentagents')).toHaveLength(1);
      await page.screenshot({ path: test.info().outputPath('overview-restored.png') });
      expect(errors).toEqual([]);
    });
  }
}

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
        await expect(dialog(page)).toContainText(
          provider === 'claude'
            ? 'An isolated provider check is unavailable'
            : 'Provider has not been checked. Authentication is unknown.',
        );
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

for (const provider of ['claude', 'codex']) {
  test(`ordinary New Agent ${provider} creates a blank chat and recovers from failure`, async ({
    page,
  }) => {
    await page.goto(`${base}?spawn=malformed`);
    await page.getByRole('button', { name: "Got it — don't show again" }).click();
    await page.keyboard.press('Control+Shift+N');
    const newAgent = page.getByRole('dialog', { name: 'New Agent', exact: true });
    await expect(newAgent).toBeVisible();
    await expect(page.getByLabel('What should this agent do?')).toHaveCount(0);
    await expect(page.getByLabel(/Allow an empty session/)).toHaveCount(0);
    await expect(page.getByLabel('Working directory')).toBeFocused();
    if (provider === 'codex')
      await newAgent.getByRole('button', { name: 'Codex', exact: true }).click();
    await page.getByLabel('Working directory').fill('/fixture/project');
    await page.screenshot({
      animations: 'disabled',
      path: test.info().outputPath('new-agent.png'),
    });
    await page.getByLabel('Working directory').press('Enter');
    await expect(page.getByRole('alert')).toContainText('could not start');
    await expect(page.getByRole('alert')).toBeFocused();
    await expect(page.getByLabel('Working directory')).toHaveValue('/fixture/project');
    await assertNoAgentWrites(page);
    await page.evaluate(() => (window as any).firstUse.spawnMode('success'));
    await newAgent.getByRole('button', { name: 'Retry launch' }).click();
    await expect(newAgent).toHaveCount(0);
    const all = await calls(page);
    const spawns = all.filter((c: any) => c.method === 'spawnClaude');
    expect(spawns).toHaveLength(2);
    for (const spawn of spawns) {
      expect(spawn.args[0].cwd).toBe('/fixture/project');
      expect(spawn.args[0].provider ?? 'claude').toBe(provider);
      expect(spawn.args[0].message).toBeUndefined();
    }
    expect(all.filter((c: any) => c.method === 'claudeMessage')).toEqual([]);
    const sessions = (await page.evaluate(() =>
      Object.values((window as any).firstUse.snapshots()),
    )) as any[];
    expect(sessions).toHaveLength(1);
    expect(sessions[0].conversation.filter((m: any) => m.role === 'user')).toEqual([]);
    const composer = page.getByPlaceholder(/^Give .+ something to do/);
    await expect(composer).toHaveValue('');
    await composer.fill('My first message after creation');
    await composer.press('Enter');
    await expect
      .poll(async () => (await calls(page)).filter((c: any) => c.method === 'claudeMessage'))
      .toEqual([
        {
          method: 'claudeMessage',
          args: [sessions[0].sessionId, 'My first message after creation'],
        },
      ]);
  });
}

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
  expect(spawns[1].args[0].message).toBeUndefined();
  await expect
    .poll(async () => (await calls(page)).filter((c: any) => c.method === 'claudeMessage').length)
    .toBe(1);
  const history = await calls(page);
  const prepared = history.filter((c: any) => c.method === 'managerRequestPrepare');
  const delivered = history.filter((c: any) => c.method === 'claudeMessage');
  expect(prepared).toHaveLength(1);
  expect(prepared[0].args[1]).toBe('Coordinate my first task');
  expect(prepared[0].args[2]).toBe(true);
  expect(delivered[0].args[0]).toBe(prepared[0].args[0]);
  expect(delivered[0].args[2]).toBe(prepared[0].args[3]);
  expect(delivered[0].args[1].split('Coordinate my first task')).toHaveLength(2);
  expect(history.indexOf(prepared[0])).toBeLessThan(history.indexOf(delivered[0]));
  expect(await page.evaluate(() => (window as any).firstUse.requests())).toEqual([
    expect.objectContaining({
      requestId: prepared[0].args[3],
      owner: delivered[0].args[0],
      text: 'Coordinate my first task',
      delivery: 'accepted',
    }),
  ]);
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

for (const state of ['starting', 'down', 'degraded', 'ready', 'adopted', 'unknown']) {
  test(`host lifecycle ${state} keeps real launch failures retryable`, async ({ page }) => {
    await page.goto(`${base}?runtime=${state}`);
    await openFirstTask(page);
    await page.getByLabel('What should this agent do?').fill('Runtime recovery task');
    if (state === 'starting' || state === 'down') {
      await expect(launch(page)).toBeDisabled();
      await expect(page.locator('#spawn-runtime-status')).toContainText(
        state === 'starting' ? 'starting' : 'unavailable',
      );
      expect((await calls(page)).filter((c: any) => c.method === 'spawnClaude')).toEqual([]);
      await page.evaluate(() => (window as any).firstUse.readiness('ready'));
      await dialog(page).getByRole('button', { name: 'Check runtime again' }).click();
    }
    await expect(launch(page)).toBeEnabled();
    await launch(page).click();
    await expect(page.getByRole('alert')).toContainText('could not start');
    await dialog(page).getByRole('button', { name: 'Check runtime again' }).click();
    await expect(page.getByRole('alert')).toContainText('could not start');
    await assertNoAgentWrites(page);
    await page.evaluate(() => (window as any).firstUse.spawnMode('success'));
    await page.getByRole('button', { name: 'Retry dispatch' }).click();
    await expect(dialog(page)).toHaveCount(0);
  });
}

test('hub-only degradation permits direct launch but blocks Fleet Manager', async ({ page }) => {
  await page.goto(`${base}?runtime=degraded`);
  await page.getByRole('button', { name: "Got it — don't show again" }).click();
  await page.getByLabel('Ask the Fleet Manager').fill('Coordinate this task');
  await expect(page.getByRole('button', { name: 'Ask Fleet Manager', exact: true })).toBeDisabled();
  await expect(page.locator('#fleet-runtime-status')).toContainText('hub or action tools failed');
  await page.evaluate(() => (window as any).firstUse.readiness('ready'));
  await page.getByRole('button', { name: 'Check runtime again' }).click();
  await expect(page.getByRole('button', { name: 'Ask Fleet Manager', exact: true })).toBeEnabled();
});

test('folder truth rejects invalid paths and drops stale git replies', async ({ page }) => {
  await page.goto(`${base}?spawn=success`);
  await openFirstTask(page);
  await page.getByLabel('What should this agent do?').fill('Folder task');
  const cwd = page.getByLabel('Working directory');
  await expect(page.locator('#spawn-folder-status')).toContainText('non-git folder');
  await cwd.fill('/fixture/git-folder');
  await expect(page.locator('#spawn-folder-status')).toContainText('main');
  await cwd.fill('/fixture/deferred');
  await expect
    .poll(
      async () =>
        (await calls(page)).filter(
          (c: any) => c.method === 'worktreeInfo' && c.args[0] === '/fixture/deferred',
        ).length,
    )
    .toBe(1);
  await cwd.fill('/fixture/invalid');
  await expect(launch(page)).toBeDisabled();
  await page.evaluate(() =>
    (window as any).firstUse.folderReply('/fixture/deferred', {
      isRepo: true,
      directory: 'accessible',
      branch: 'stale-branch',
    }),
  );
  await expect(page.locator('#spawn-folder-status')).toContainText('Choose an existing');
  await expect(page.locator('#spawn-folder-status')).not.toContainText('stale-branch');
  await expect(launch(page)).toBeDisabled();
  await cwd.fill('~');
  await expect(page.locator('#spawn-folder-status')).toContainText('not expanded');
  await expect(launch(page)).toBeDisabled();
  await assertNoAgentWrites(page);
});

test('remote cwd never borrows local folder or runtime facts', async ({ page }) => {
  await page.goto(`${base}?remote=1&runtime=ready&spawn=success`);
  await openFirstTask(page);
  await page.getByLabel('What should this agent do?').fill('Remote task');
  await page.getByRole('button', { name: /advanced/i }).click();
  const machine = dialog(page)
    .getByRole('combobox')
    .filter({ has: page.getByRole('option', { name: 'This machine', exact: true }) });
  await machine.selectOption('Remote fixture');
  await page.getByLabel('Working directory').fill('/remote/owner-only');
  await expect(page.locator('#spawn-folder-status')).toContainText('selected remote machine');
  await expect(page.locator('#spawn-runtime-status')).toContainText('unknown');
  await page.getByRole('button', { name: 'Check folder again' }).click();
  expect(
    (await calls(page)).filter(
      (c: any) => c.method === 'worktreeInfo' && c.args[0] === '/remote/owner-only',
    ),
  ).toEqual([]);
  await expect(
    dialog(page).getByRole('button', { name: 'Browse…', exact: true }).first(),
  ).toBeDisabled();
  await launch(page).click();
  const spawn = (await calls(page)).find((c: any) => c.method === 'spawnClaude');
  expect(spawn.args[0]).toMatchObject({ cwd: '/remote/owner-only', targetHub: 'Remote fixture' });
  expect(spawn.args[0].worktree).toBeUndefined();
});

for (const theme of ['light', 'dracula']) {
  for (const width of [360, 1280]) {
    test(`first conversation result and restart ${theme} ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(`${base}?theme=${theme}&spawn=success&runtime=ready`);
      await openFirstTask(page);
      const task = 'Explain the first result';
      await page.getByLabel('What should this agent do?').fill(task);
      await launch(page).click();
      await expect(page.getByText(task, { exact: true }).first()).toBeVisible();
      await expect(page.getByRole('complementary', { name: 'First task guidance' })).toBeVisible();
      await page.getByRole('button', { name: 'Dismiss task guidance' }).focus();
      await page.keyboard.press('Enter');
      await expect(page.getByRole('complementary', { name: 'First task guidance' })).toHaveCount(0);
      const id = await page.evaluate(() => Object.keys((window as any).firstUse.snapshots())[0]);
      await page.evaluate(
        (id) => (window as any).firstUse.update(id, { ambientState: 'streaming' }),
        id,
      );
      await page.evaluate(
        (id) =>
          (window as any).firstUse.update(id, {
            ambientState: 'waiting_input',
            pendingQuestions: [
              {
                question: 'Choose the next step',
                header: 'Decision',
                options: [{ label: 'Continue', description: 'Explain the result' }],
              },
            ],
          }),
        id,
      );
      await expect(page.getByText('Choose the next step', { exact: true }).first()).toBeVisible();
      await page.evaluate(
        (id) =>
          (window as any).firstUse.update(id, { ambientState: 'idle', pendingQuestions: null }),
        id,
      );
      await page.evaluate(
        (id) => (window as any).firstUse.reply(id, 'I need your decision before continuing.'),
        id,
      );
      await expect(
        page.getByText('I need your decision before continuing.', { exact: true }).first(),
      ).toBeVisible();
      await page.evaluate(
        (id) =>
          (window as any).firstUse.reply(
            id,
            'The explanation is complete.\n\n```wks-result\n{"commit":"fixture-only","checksRun":["mock check"]}\n```',
          ),
        id,
      );
      await expect(
        page.getByText('The explanation is complete.', { exact: true }).first(),
      ).toBeVisible();
      await expect(page.getByText(/fixture-only/).first()).toBeVisible();
      await page.evaluate(
        (id) =>
          (window as any).firstUse.reply(
            id,
            'Malformed payload remains visible.\n\n```wks-result\n{ broken json\n```',
          ),
        id,
      );
      await expect(
        page.getByText('Malformed payload remains visible.', { exact: true }).first(),
      ).toBeVisible();
      await page.evaluate(
        (id) =>
          (window as any).firstUse.reply(
            id,
            'Plain prose is still useful; no structured result was supplied.',
          ),
        id,
      );
      await expect(
        page
          .getByText('Plain prose is still useful; no structured result was supplied.', {
            exact: true,
          })
          .first(),
      ).toBeVisible();
      await expect
        .poll(() =>
          page.evaluate(
            () => (window as any).firstUse.layout()?.agents.filter((a: any) => !a.global).length,
          ),
        )
        .toBe(1);
      await page.evaluate(() => (window as any).firstUse.restart());
      await page.reload();
      await expect(page.getByRole('dialog', { name: 'Welcome' })).toHaveCount(0);
      await expect(
        page
          .getByText('Plain prose is still useful; no structured result was supplied.', {
            exact: true,
          })
          .first(),
      ).toBeVisible();
      await expect
        .poll(async () => (await calls(page)).filter((c: any) => c.method === 'spawnClaude').length)
        .toBe(1);
      const resumed = (await calls(page)).filter((c: any) => c.method === 'spawnClaude');
      expect(resumed[0].args[0].resumeSessionId).toBe(id);
      expect(await page.evaluate(() => Object.keys((window as any).firstUse.snapshots()))).toEqual([
        id,
      ]);
      await expect(page.getByRole('complementary', { name: 'First task guidance' })).toHaveCount(0);
      await page.screenshot({
        animations: 'disabled',
        path: test.info().outputPath('resumed-first-result.png'),
      });
    });
  }
}

test('first managed chat distinguishes escalation, results and missing contracts', async ({
  page,
}) => {
  await page.goto(`${base}?spawn=success&runtime=ready`);
  await page.getByRole('button', { name: "Got it — don't show again" }).click();
  await page.getByLabel('Ask the Fleet Manager').fill('Coordinate a fixture task');
  await page.getByRole('button', { name: 'Ask Fleet Manager', exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => Object.keys((window as any).firstUse.snapshots()).length))
    .toBe(1);
  const id = await page.evaluate(() => Object.keys((window as any).firstUse.snapshots())[0]);
  await page.evaluate(
    (id) =>
      (window as any).firstUse.wake(id, 'worker-escalated', {
        label: 'Fixture worker',
        sessionId: 'fixture-worker',
        cwd: '/fixture/project',
        escalation: JSON.stringify({
          type: 'worker-escalation',
          status: 'blocked',
          reason: 'Choose the target release',
          requiredAuthorityOrDecision: 'Release target',
          changed: false,
          nextAction: 'Name the release',
        }),
      }),
    id,
  );
  await expect(page.getByText('worker escalation', { exact: true }).first()).toBeVisible();
  await expect(page.getByText('Choose the target release', { exact: true }).first()).toBeVisible();
  await expect(page.getByText('structured result', { exact: true })).toHaveCount(0);
  await page.evaluate(
    (id) =>
      (window as any).firstUse.wake(id, 'worker-finished', {
        label: 'Fixture worker',
        sessionId: 'fixture-worker',
        cwd: '/fixture/project',
        lastReply: 'Fixture work is complete.',
        result: JSON.stringify({ commit: 'abc12345', checksRun: ['Fixture checks pass'] }),
      }),
    id,
  );
  await expect(page.getByText('structured result', { exact: true }).first()).toBeVisible();
  await expect(page.getByLabel('Copy commit abc12345').first()).toBeVisible();
  await page.evaluate(
    (id) =>
      (window as any).firstUse.wake(id, 'worker-finished', {
        label: 'Prose worker',
        sessionId: 'fixture-prose',
        cwd: '/fixture/project',
        lastReply: 'The prose report is retained.',
        resultError: 'No wks-result block was supplied',
      }),
    id,
  );
  await expect(
    page.getByText('No wks-result block was supplied', { exact: true }).first(),
  ).toBeVisible();
  await page.evaluate(
    (id) =>
      (window as any).firstUse.wake(id, 'worker-finished', {
        label: 'Malformed worker',
        sessionId: 'fixture-malformed',
        cwd: '/fixture/project',
        lastReply: 'Malformed result still has readable prose.',
        resultError: 'Result JSON could not be parsed',
      }),
    id,
  );
  await expect(
    page.getByText('Result JSON could not be parsed', { exact: true }).first(),
  ).toBeVisible();
  await page.getByText('last reply', { exact: true }).last().click();
  await expect(
    page.getByText('Malformed result still has readable prose.', { exact: true }).first(),
  ).toBeVisible();
});

test('CLI install notice opens the actual Command Line settings section', async ({ page }) => {
  await page.goto(base);
  await page.getByRole('button', { name: "Got it — don't show again" }).click();
  await page.evaluate(() =>
    (window as any).firstUse.notice({
      level: 'info',
      key: 'cli-install',
      title: 'workspacer command installed',
    }),
  );
  await page.getByRole('button', { name: 'Settings → Command Line', exact: true }).click();
  await expect(page.getByText('workspacer serve', { exact: true }).first()).toBeVisible();
});

test('first-task help reaches the actual routing and workflow settings without dispatching', async ({
  page,
}) => {
  await page.goto(`${base}?spawn=success&runtime=ready`);
  await openFirstTask(page);
  await page.getByLabel('What should this agent do?').fill('Explain this fixture');
  await launch(page).click();
  const guidance = page.getByRole('complementary', { name: 'First task guidance' });
  await expect(guidance).toBeVisible();
  const before = (await calls(page)).filter((c: any) => c.method === 'spawnClaude').length;
  await guidance.getByRole('button', { name: 'Model routing', exact: true }).click();
  const routing = page.getByRole('region', { name: 'Routing settings', exact: true });
  await expect(routing).toBeVisible();
  await routing.getByRole('button', { name: 'Fleet workflows', exact: true }).click();
  const workflows = page.getByRole('region', { name: 'Fleet workflows', exact: true });
  await expect(workflows).toBeVisible();
  await workflows.getByRole('button', { name: 'Model routing', exact: true }).click();
  await expect(routing).toBeVisible();
  expect((await calls(page)).filter((c: any) => c.method === 'spawnClaude')).toHaveLength(before);
  expect(await page.evaluate(() => Object.keys((window as any).firstUse.snapshots()).length)).toBe(
    1,
  );
});

for (const state of ['responding', 'unauthenticated', 'unsupported', 'error']) {
  test(`provider readiness ${state} is advisory and runtime stays health-only`, async ({
    page,
  }) => {
    await page.goto(`${base}?runtime=ready&providerReadiness=${state}`);
    await page.getByRole('button', { name: "Got it — don't show again" }).click();
    const expected = {
      responding: 'Provider responded to a small test request.',
      unauthenticated: 'Provider reported an authentication failure.',
      unsupported: 'An isolated provider check is unavailable',
      error: 'Provider check failed; authentication is unknown.',
    }[state]!;
    await expect(page.locator('#fleet-provider-status')).toContainText(expected);
    await expect(page.locator('#fleet-runtime-status')).toContainText(
      'Fleet Manager runtime is ready.',
    );
    await expect(page.locator('#fleet-runtime-status')).not.toContainText('sign-in');
    await page.getByLabel('Ask the Fleet Manager').fill('Fixture request');
    await expect(
      page.getByRole('button', { name: 'Ask Fleet Manager', exact: true }),
    ).toBeEnabled();
    const before = await calls(page);
    expect(
      before.filter((c: any) => c.method === 'providerReadiness' && c.args[1] === true),
    ).toEqual([]);
    await page
      .locator('#fleet-provider-status')
      .getByRole('button', { name: 'Check again' })
      .click();
    expect(
      (await calls(page)).filter(
        (c: any) => c.method === 'providerReadiness' && c.args[1] === true,
      ),
    ).toHaveLength(1);
  });
}

test('Codex readiness success stays on Codex and never adds a launch gate', async ({ page }) => {
  await page.goto(`${base}?managerProvider=codex&runtime=ready&providerReadiness=responding`);
  await page.getByRole('button', { name: "Got it — don't show again" }).click();
  await expect(page.locator('#fleet-provider-status')).toContainText('codex CLI found.');
  await expect(page.locator('#fleet-provider-status')).toContainText(
    'Provider responded to a small test request.',
  );
  await page.getByLabel('Ask the Fleet Manager').fill('Fixture task');
  await expect(page.getByRole('button', { name: 'Ask Fleet Manager', exact: true })).toBeEnabled();
  await page.locator('#fleet-provider-status').getByRole('button', { name: 'Check again' }).click();
  expect(
    (await calls(page))
      .filter((c: any) => c.method === 'providerReadiness' && c.args[1] === true)
      .map((c: any) => c.args[0]),
  ).toEqual(['codex']);
});

for (const capture of ['legacy', 'missing', 'remote']) {
  test(`manager bootstrap delivers one ask with explicit ${capture} capture capability`, async ({ page }) => {
    await page.goto(`${base}?capture=${capture}&spawn=success`);
    await page.getByRole('button', { name: "Got it — don't show again" }).click();
    const ask = `One ${capture} manager request`;
    await page.getByRole('textbox', { name: 'Ask the Fleet Manager' }).fill(ask);
    await page.getByRole('textbox', { name: 'Ask the Fleet Manager' }).press('Enter');
    await expect.poll(async () => {
      const history = await calls(page);
      return history.filter((c: any) =>
        (c.method === 'spawnClaude' && c.args[0].message?.includes(ask)) ||
        (c.method === 'claudeMessage' && c.args[1]?.includes(ask))).length;
    }).toBe(1);
    const history = await calls(page);
    const spawns = history.filter((c: any) => c.method === 'spawnClaude');
    const sends = history.filter((c: any) => c.method === 'claudeMessage');
    expect(spawns).toHaveLength(1);
    expect(spawns[0].args[0]).toMatchObject({ manager: true, transport: 'stream' });
    expect(history.filter((c: any) => c.method === 'managerRequestPrepare')).toHaveLength(0);
    expect(await page.evaluate(() => (window as any).firstUse.requests())).toEqual([]);
    if (capture === 'remote') {
      expect(spawns[0].args[0].message).toBeUndefined();
      expect(sends).toHaveLength(1);
      expect(sends[0].args).toHaveLength(2);
    } else {
      expect(spawns[0].args[0].message.split(ask)).toHaveLength(2);
      expect(sends).toHaveLength(0);
    }
  });
}
