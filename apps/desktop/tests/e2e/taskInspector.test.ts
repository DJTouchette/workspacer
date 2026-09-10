/** Real Chromium and production RecentAgentsPane + bridged backend.
 * IPC history/evidence and bus snapshots are explicit fixtures; no live agents. */
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
  console.info('Task Inspector Chromium:', browser.version());
  const port = await freePort();
  base = `http://127.0.0.1:${port}/task-inspector-harness.html`;
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
  throw new Error('Task Inspector harness failed to start');
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
  test(`future required review skip and recorded work at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto(`${base}?worker=worker`);
    // Identifiers are not in the default view: the raw task id and the absolute
    // project path exist only behind the Details disclosure.
    await expect(page.getByText('task-current').first()).toBeHidden();
    await expect(page.getByText('/project/alpha').first()).toBeHidden();
    await page.getByText('Filters', { exact: true }).click();
    await expect(
      page.getByText('Tasks linked to worker worker by a recorded attempt.'),
    ).toBeVisible();
    await page.getByText('Details', { exact: true }).click();
    await expect(page.getByText('task-current').first()).toBeVisible();
    await expect(page.getByText('/project/alpha').first()).toBeVisible();
    await expect(page.getByText('Worker worker', { exact: false }).first()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Skip implement…' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Skip review…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Skip task step' });
    await expect(
      dialog.getByText('This skips the required independent review for this task.'),
    ).toBeVisible();
    await dialog.getByLabel('Reason (optional)').fill('Review not needed for this task');
    await dialog.getByRole('button', { name: 'Skip this step' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByText(/Skipped by you/).first()).toBeVisible();
    // A finished step offers no skip affordance at all — not a disabled one.
    await expect(page.getByRole('button', { name: 'Skip review…' })).toHaveCount(0);
    const task = await page.evaluate(() => (window as any).fixtureTask);
    expect(task.workflow.steps.map((s: any) => s.state)).toEqual([
      'skipped',
      'dispatched',
      'waived',
    ]);
    expect(task.workflow.hash).toBe('fixture-pin');
    expect(task.attempts[0].resultContract).toBe('absent');
    await page.getByRole('button', { name: 'Open worktree' }).click();
    expect(await page.evaluate(() => (window as any).taskCalls.at(-1))).toEqual({
      taskId: 'task-current',
      kind: 'worktree',
      dispatchId: 'attempt',
    });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({
      path: test.info().outputPath(`task-inspector-${width}.png`),
      fullPage: true,
    });
  });
test('save references, safe selectors, preserve conflict draft and deep-link actual project workflow setting', async ({
  page,
}) => {
  await page.goto(base);
  await page.getByRole('button', { name: 'Links' }).click();
  await page.getByLabel('PR number', { exact: true }).fill('123');
  await page.getByLabel('PR URL', { exact: true }).fill('https://example.test/pr/123');
  await page.getByRole('button', { name: 'Add ticket', exact: true }).click();
  await page.getByLabel('Ticket ID', { exact: true }).fill('WKS-9');
  await page.getByRole('button', { name: 'Save references' }).click();
  // The saved link surfaces as a chip at the top, not as another form row.
  await expect(page.getByRole('button', { name: 'PR 123' })).toBeVisible();
  await page.getByRole('button', { name: 'PR 123' }).click();
  expect(await page.evaluate(() => (window as any).taskCalls.at(-1))).toEqual({
    taskId: 'task-current',
    kind: 'url',
    reference: 'pullRequest',
  });
  await page.getByLabel('PR number', { exact: true }).fill('456');
  await page.evaluate(() => (window as any).fixtureChange());
  await page.getByRole('button', { name: 'Save references' }).click();
  await expect(page.getByText(/Your draft is preserved/)).toBeVisible();
  await expect(page.getByLabel('PR number', { exact: true })).toHaveValue('456');
  await page.getByRole('button', { name: 'Keep draft on current task' }).click();
  await page.getByRole('button', { name: 'Save references' }).click();
  expect(await page.evaluate(() => (window as any).fixtureTask.ownerLabel)).toBe(
    'Replacement manager',
  );
  await page.getByText('Details', { exact: true }).click();
  await page.getByRole('button', { name: 'Configure workflow', exact: true }).click();
  await expect(page.getByText('New tasks in /project/alpha')).toBeVisible();
  await page
    .getByLabel('Fleet workflow for /project/alpha', { exact: true })
    .selectOption('direct-implementation');
  expect(
    await page.evaluate(() => (window as any).taskCalls.findLast((r: any) => r.op === 'select')),
  ).toEqual({
    op: 'select',
    cwd: '/project/alpha',
    workflowId: 'direct-implementation',
    expectedRevision: 4,
  });
  expect(await page.evaluate(() => (window as any).fixtureTask.workflow.hash)).toBe('fixture-pin');
});
test('failure evidence survives an explicit skip', async ({ page }) => {
  await page.goto(`${base}?mode=failed`);
  await expect(page.getByText('Worker result contract: invalid').first()).toBeVisible();
  await page.getByRole('button', { name: 'Skip implement…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Skip this step' }).click();
  // The step is finished now, so its rationale and reported outcome collapse —
  // they are preserved, not deleted.
  await expect(page.getByText('Worker result contract: invalid').first()).toBeHidden();
  await page.getByText('Step details', { exact: true }).nth(1).click();
  await expect(page.getByText('Worker result contract: invalid').first()).toBeVisible();
  await expect(page.getByText('Original failure evidence').first()).toBeVisible();
  expect(await page.evaluate(() => (window as any).fixtureTask.workflow.steps[1].outcome)).toEqual({
    failure: 'Original failure evidence',
  });
});
for (const [mode, message] of [
  ['empty', 'No recorded tasks match this selection.'],
  ['old', 'Recent agents history is available only'],
  ['remote', 'Task Inspector is available in the local desktop app only'],
  ['error', 'Could not refresh tasks.'],
  ['loading', 'Loading tasks…'],
  ['no-manager', 'No current manager.'],
] as const)
  test(`honest ${mode} state`, async ({ page }) => {
    await page.goto(`${base}?mode=${mode}`);
    await expect(page.getByText(message, { exact: false }).first()).toBeVisible();
  });
test('unknown worker attribution stays empty and recent tasks remain selectable', async ({
  page,
}) => {
  await page.goto(`${base}?worker=unrecorded`);
  await expect(page.getByText(/No recorded tasks match/)).toBeVisible();
  await page.getByText('Filters', { exact: true }).click();
  await page.getByLabel('All recorded tasks', { exact: true }).check();
  await page.getByLabel('Current and recent tasks', { exact: true }).selectOption('recent');
  await expect(page.getByText('No workflow was recorded for this task.')).toBeVisible();
  // Short manager label by default; the raw owner session id is under Details.
  await expect(page.getByText(/Previous manager · beta/)).toBeVisible();
  await page.getByText('Details', { exact: true }).click();
  await expect(page.getByText('old-manager').first()).toBeVisible();
});
test('stale worker skip is unavailable and task routing selects the explicit task', async ({
  page,
}) => {
  await page.goto(`${base}?mode=stale`);
  await expect(page.getByRole('button', { name: 'Skip implement…' })).toHaveCount(0);
  await expect(page.getByText(/Worker status is unknown/)).toBeHidden();
  await page.getByText('Step details', { exact: true }).last().click();
  await expect(page.getByText(/Worker status is unknown/)).toBeVisible();
  await page.goto(`${base}?mode=route`);
  await page.getByRole('button', { name: 'Inspect recent task' }).click();
  await expect(page.getByLabel('Current and recent tasks', { exact: true })).toHaveValue('recent');
});

test('stale skip confirmation preserves the reason and refuses a newly dispatched step', async ({
  page,
}) => {
  await page.goto(base);
  await page.getByRole('button', { name: 'Skip review…' }).click();
  await page.getByRole('dialog').getByLabel('Reason (optional)').fill('My task-specific reason');
  await page.evaluate(() => {
    const t = (window as any).fixtureTask;
    t.revision++;
    t.workflow.steps[2].state = 'dispatched';
  });
  await expect(
    page.getByRole('dialog').getByRole('button', { name: 'Refresh confirmation' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Refresh confirmation' }).click();
  await expect(
    page.getByRole('dialog').getByRole('button', { name: 'Skip this step' }),
  ).toBeDisabled();
  await expect(page.getByRole('dialog').getByLabel('Reason (optional)')).toHaveValue(
    'My task-specific reason',
  );
  expect(await page.evaluate(() => (window as any).taskCalls)).toEqual([]);
});
test('reference validation refuses unsafe schemes and supports user-named http references', async ({
  page,
}) => {
  await page.goto(base);
  await page.getByRole('button', { name: 'Links' }).click();
  await page.getByLabel('PR URL', { exact: true }).fill('file:///etc/passwd');
  await page.getByRole('button', { name: 'Save references' }).click();
  await expect(page.getByRole('alert')).toContainText('Only http(s)');
  expect(await page.evaluate(() => (window as any).taskCalls)).toEqual([]);
  await page.getByLabel('PR URL', { exact: true }).fill('');
  await page.getByRole('button', { name: 'Add reference', exact: true }).click();
  await page.getByLabel('Reference label', { exact: true }).fill('Design notes');
  await page.getByLabel('Reference URL', { exact: true }).fill('https://example.test/design');
  await page.getByRole('button', { name: 'Save references' }).click();
  await page.getByRole('button', { name: 'Design notes' }).click();
  expect(await page.evaluate(() => (window as any).taskCalls.at(-1))).toEqual({
    taskId: 'task-current',
    kind: 'url',
    reference: 'references',
    index: 0,
  });
});

/* Screenshots of the real component at the widths the rail actually uses, in
 * three themes. These are for human review of density and brand fit; the test
 * itself only asserts that nothing overflows horizontally. */
for (const theme of ['everforest', 'dark', 'light'])
  for (const width of [360, 480, 1280])
    test(`compact layout at ${width}px in ${theme}`, async ({ page }, info) => {
      await page.setViewportSize({ width, height: 1100 });
      await page.goto(`${base}?mode=links&theme=${theme}&width=${width}`);
      await expect(page.getByRole('heading', { name: 'Build Task Inspector' })).toBeVisible();
      await expect(page.getByRole('button', { name: 'PR 9492' })).toBeVisible();
      await expect(page.getByRole('button', { name: 'WKS-412' })).toBeVisible();
      // SUP-77 has no URL, so it is a plain chip rather than a link button.
      await expect(page.getByRole('button', { name: 'SUP-77' })).toHaveCount(0);
      await expect(page.getByText('SUP-77')).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({
        path: path.resolve(__dirname, `../../test-results/task-inspector/${theme}-${width}.png`),
        fullPage: true,
      });
      info.annotations.push({ type: 'screenshot', description: `${theme}-${width}` });
    });

test('a reference recorded outside this panel appears on the next refresh', async ({ page }) => {
  await page.goto(base);
  await expect(page.getByText('No links recorded yet.')).toBeHidden();
  // Stands in for a manager writing the task reference over MCP while the
  // panel is open: the Inspector polls dispatch history and must pick it up.
  await page.evaluate(() => {
    const t = (window as any).fixtureTask;
    t.revision++;
    t.links = { pullRequest: { number: '9492', url: 'https://dev.azure.test/pullrequest/9492' } };
  });
  await expect(page.getByRole('button', { name: 'PR 9492' })).toBeVisible({ timeout: 10000 });
  await page.getByRole('button', { name: 'PR 9492' }).click();
  expect(await page.evaluate(() => (window as any).taskCalls.at(-1))).toEqual({
    taskId: 'task-current',
    kind: 'url',
    reference: 'pullRequest',
  });
});

for (const theme of ['everforest', 'dark', 'light'])
  test(`working step at 360px in ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width: 360, height: 800 });
    await page.goto(`${base}?theme=${theme}&width=360`);
    await expect(page.getByRole('button', { name: 'Skip implement…' })).toHaveCount(0);
    await expect(page.getByText('A worker has been dispatched for this step')).toBeHidden();
    await page.screenshot({
      path: path.resolve(__dirname, `../../test-results/task-inspector/${theme}-working-360.png`),
      fullPage: true,
    });
    await page.getByText('Step details', { exact: true }).last().click();
    await expect(page.getByText('A worker has been dispatched for this step')).toBeVisible();
  });
