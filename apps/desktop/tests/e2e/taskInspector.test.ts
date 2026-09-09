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
    await expect(
      page.getByText('Tasks linked to worker worker by a recorded attempt.'),
    ).toBeVisible();
    await expect(page.getByText('Worker worker', { exact: false }).first()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Skip implement…' })).toBeDisabled();
    await page.getByRole('button', { name: 'Skip review…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Skip task step' });
    await expect(
      dialog.getByText('This skips the required independent review for this task.'),
    ).toBeVisible();
    await dialog.getByLabel('Reason (optional)').fill('Review not needed for this task');
    await dialog.getByRole('button', { name: 'Skip this step' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByText(/Skipped by you/).first()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Skip review…' })).toBeDisabled();
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
  await page.getByLabel('PR number', { exact: true }).fill('123');
  await page.getByLabel('PR URL', { exact: true }).fill('https://example.test/pr/123');
  await page.getByRole('button', { name: 'Add ticket', exact: true }).click();
  await page.getByLabel('Ticket ID', { exact: true }).fill('WKS-9');
  await page.getByRole('button', { name: 'Save references' }).click();
  await expect(page.getByRole('button', { name: 'Open PR' })).toBeVisible();
  await page.getByRole('button', { name: 'Open PR' }).click();
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
  await page.getByRole('button', { name: 'Skip implement…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Skip this step' }).click();
  await expect(page.getByText('Worker result contract: invalid').first()).toBeVisible();
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
  await page.getByLabel('All recorded tasks', { exact: true }).check();
  await page.getByLabel('Current and recent tasks', { exact: true }).selectOption('recent');
  await expect(page.getByText('No workflow was recorded for this task.')).toBeVisible();
  await expect(page.getByText(/Previous manager \(old-manager\)/)).toBeVisible();
});
test('stale worker skip is disabled and task routing selects the explicit task', async ({
  page,
}) => {
  await page.goto(`${base}?mode=stale`);
  await expect(page.getByRole('button', { name: 'Skip implement…' })).toBeDisabled();
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
  await page.getByLabel('PR URL', { exact: true }).fill('file:///etc/passwd');
  await page.getByRole('button', { name: 'Save references' }).click();
  await expect(page.getByRole('alert')).toContainText('Only http(s)');
  expect(await page.evaluate(() => (window as any).taskCalls)).toEqual([]);
  await page.getByLabel('PR URL', { exact: true }).fill('');
  await page.getByRole('button', { name: 'Add reference', exact: true }).click();
  await page.getByLabel('Reference label', { exact: true }).fill('Design notes');
  await page.getByLabel('Reference URL', { exact: true }).fill('https://example.test/design');
  await page.getByRole('button', { name: 'Save references' }).click();
  await page.getByRole('button', { name: 'Open Design notes' }).click();
  expect(await page.evaluate(() => (window as any).taskCalls.at(-1))).toEqual({
    taskId: 'task-current',
    kind: 'url',
    reference: 'references',
    index: 0,
  });
});
