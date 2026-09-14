/** Production App + real private desktop host. Only provider/session activity is
 * mocked; all intent records, artifacts, Git capture and reviewed learning writes
 * use isolated on-disk storage. No credentials or live daemon URLs are inherited. */
import { test, expect, type Page } from '@playwright/test';
import { spawn, execFileSync, type ChildProcess } from 'node:child_process';
import { createInterface } from 'node:readline';
import { once } from 'node:events';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { freePort } from './fixtures/scratchState';

let vite: ChildProcess;
let base: string;
let cache: string;
test.beforeAll(async () => {
  const port = await freePort();
  base = `http://127.0.0.1:${port}/first-use-harness.html`;
  cache = fs.mkdtempSync(path.join(os.tmpdir(), 'intent-completion-vite-'));
  vite = spawn(
    process.execPath,
    [
      '--input-type=module',
      '-e',
      `import { createServer } from 'vite'; const server = await createServer(${JSON.stringify({ cacheDir: cache, server: { host: '127.0.0.1', port, strictPort: true } })}); await server.listen();`,
    ],
    { cwd: path.resolve(__dirname, '../../src/renderer'), stdio: 'ignore' },
  );
  for (const deadline = Date.now() + 20000; Date.now() < deadline;) {
    try {
      if ((await fetch(base)).ok) return;
    } catch {
      /* Vite is starting. */
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  throw new Error('Intent completion harness failed to start');
});
test.afterAll(async () => {
  if (vite && vite.exitCode === null && vite.signalCode === null) {
    const stopped = once(vite, 'exit');
    vite.kill();
    await stopped;
  }
  if (cache) fs.rmSync(cache, { recursive: true, force: true });
});

function isolatedHost(repo: string, env: NodeJS.ProcessEnv, callbacks: any[]) {
  const child = spawn(
    process.execPath,
    [path.resolve(__dirname, '../../dist/headless/desktop-host.cjs')],
    { cwd: path.resolve(__dirname, '../..'), env, stdio: ['pipe', 'pipe', 'pipe'] },
  );
  let log = '',
    sequence = 0;
  const pending = new Map<
    string,
    { resolve: (value: any) => void; reject: (error: Error) => void; timeout: NodeJS.Timeout }
  >();
  const stopped = once(child, 'close');
  child.stderr.on('data', (chunk) => {
    log = (log + String(chunk)).slice(-8000);
  });
  createInterface({ input: child.stdout }).on('line', (line) => {
    const response = JSON.parse(line);
    if (response.event) return;
    if (response.hostCallId) {
      callbacks.push(response);
      const allowed = response.method === 'intent.interrupt' || response.method === 'intent.send';
      child.stdin.write(
        JSON.stringify({
          hostResultId: response.hostCallId,
          ...(allowed
            ? {
                result: {
                  status: 'accepted',
                  detail: 'Isolated test transport accepted the request',
                },
              }
            : { error: 'No external lifecycle action is allowed in this test' }),
        }) + '\n',
      );
      return;
    }
    const entry = pending.get(response.id);
    if (!entry) return;
    pending.delete(response.id);
    clearTimeout(entry.timeout);
    response.error ? entry.reject(new Error(response.error)) : entry.resolve(response.result);
  });
  child.on('exit', () => {
    for (const entry of pending.values()) {
      clearTimeout(entry.timeout);
      entry.reject(new Error(`Isolated host exited: ${log}`));
    }
    pending.clear();
  });
  return {
    request(request: any, snapshots: any[] = []): Promise<any> {
      return new Promise((resolve, reject) => {
        const id = String(++sequence);
        const timeout = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`Host request timed out: ${request.action}\n${log}`));
        }, 15000);
        pending.set(id, { resolve, reject, timeout });
        child.stdin.write(
          JSON.stringify({
            id,
            method: 'desktop.intentWorkspaceRequest',
            params: { request },
            context: {
              workspaceRoots: [repo],
              setupRoots: [repo],
              snapshots,
              templates: [],
              recent: [{ cwd: repo }],
            },
          }) + '\n',
        );
      });
    },
    async stop() {
      if (child.exitCode === null && child.signalCode === null) child.stdin.end();
      const fallback = setTimeout(() => child.kill(), 3000);
      await stopped;
      clearTimeout(fallback);
    },
  };
}
const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const work = (page: Page) => page.getByRole('button', { name: 'Work', exact: true });

test('completes the intent review workflow through persistent owner services on desktop and mobile', async ({
  page,
}, testInfo) => {
  test.setTimeout(120000);
  page.setDefaultTimeout(15000);
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'intent-completion-'));
  const taskHome = path.join(root, 'home'),
    repo = path.join(taskHome, 'project');
  fs.mkdirSync(path.join(repo, '.rivet/context/modules'), { recursive: true });
  const env: NodeJS.ProcessEnv = {
    PATH: process.env.PATH,
    HOME: taskHome,
    USERPROFILE: taskHome,
    XDG_CONFIG_HOME: path.join(root, 'config'),
    XDG_DATA_HOME: path.join(root, 'data'),
    XDG_CACHE_HOME: path.join(root, 'cache'),
    APPDATA: path.join(root, 'config'),
    CLAUDE_CONFIG_DIR: path.join(taskHome, '.claude'),
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: '/dev/null',
    LANG: 'C.UTF-8',
  };
  const git = (...args: string[]) =>
    execFileSync('git', args, { cwd: repo, env, encoding: 'utf8' });
  git('init', '-q');
  git('config', 'user.name', 'Intent Fixture');
  git('config', 'user.email', 'intent@example.invalid');
  git('config', 'commit.gpgsign', 'false');
  fs.writeFileSync(path.join(repo, 'export.txt'), 'Original export\n');
  fs.writeFileSync(
    path.join(repo, '.rivet/context/modules/sample.md'),
    '# Export permissions\n\nPreserve the existing permission boundary.\n',
  );
  git('add', '.');
  git('commit', '-qm', 'Fixture baseline');
  fs.writeFileSync(path.join(repo, 'export.txt'), 'Filtered export preserves permissions\n');
  const callbacks: any[] = [],
    requests: string[] = [],
    browserErrors: string[] = [];
  let host = isolatedHost(repo, env, callbacks);
  try {
    page.on('pageerror', (error) => {
      browserErrors.push(error.message);
      console.error('Intent browser error:', error.stack);
    });
    await page.route('**/*', (route) => {
      const url = new URL(route.request().url());
      return url.origin === new URL(base).origin || ['data:', 'blob:'].includes(url.protocol)
        ? route.continue()
        : route.abort();
    });
    await page.exposeFunction('intentTestHost', (request: any, snapshots: any[]) => {
      requests.push(request.action);
      return host.request(request, snapshots);
    });
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto(`${base}?spawn=success&runtime=ready`);
    await page.getByRole('button', { name: "Got it — don't show again" }).click();
    const sessionId = await page.evaluate(async (repo) => {
      const api = (window as any).electronAPI;
      api.claudeApprove = async (id: string, decision: string) => {
        const fixture = (window as any).firstUse;
        fixture.calls.push({ method: 'claudeApprove', args: [id, decision] });
        fixture.update(id, { ambientState: 'idle', pendingApproval: null });
      };
      api.intentWorkspaceRequest = (request: any) =>
        (window as any).intentTestHost(
          request,
          Object.values((window as any).firstUse.snapshots()),
        );
      api.usagePacingSchedule = async () => null;
      await api.saveConfig({
        ui: { intentWorkspaces: true },
        agents: { defaultProvider: 'codex', spawnInWorktree: false },
      });
      return api.spawnClaude({
        cwd: repo,
        provider: 'codex',
        label: 'Export worker',
        message: 'Fixture agent; no real process exists',
      });
    }, repo);
    await work(page).click();
    await page.getByRole('button', { name: 'Create an intent' }).click();
    await page.getByLabel('Project directory').fill(repo);
    await page.getByLabel('Title', { exact: true }).fill('Export filtered rows');
    await page
      .getByLabel('Desired outcome')
      .fill('Customers export only rows matching their filters.');
    await page.getByLabel('Constraints').fill('Preserve authorization');
    await page.getByLabel('Success criteria').fill('Filtered exports preserve permissions');
    await page.getByRole('button', { name: 'Create workspace' }).click();
    await expect(page.getByText('Saved revision 1.', { exact: true })).toBeVisible();
    await tab(page, 'Intent').click();
    await page.getByLabel('Constraints').fill('Preserve authorization and column headers');
    await page.getByRole('button', { name: 'Back to agents' }).click();
    await work(page).click();
    await expect(page.getByLabel('Constraints')).toHaveValue(
      'Preserve authorization and column headers',
    );
    await page.getByLabel('Reason for this revision').fill('Clarified CSV format');
    await page.getByRole('button', { name: 'Save revision' }).click();
    await expect(page.getByText('Saved revision 2.', { exact: true })).toBeVisible();
    const workspace = (await host.request({ action: 'list' })).workspaces[0];

    await tab(page, 'Execution').click();
    await page.getByLabel('Existing agent').selectOption(JSON.stringify(['', sessionId]));
    await page.getByRole('button', { name: 'Link agent', exact: true }).click();
    await expect(page.getByText(/Tracking link/)).toBeVisible();
    const execution = (await host.request({ action: 'executions', id: workspace.id }))
      .executions[0];
    await tab(page, 'Review').click();
    await page.getByLabel('Success criterion').selectOption('r2:c1');
    await page.getByLabel('Related execution').selectOption(execution.id);
    await page.getByRole('button', { name: 'Capture Git evidence' }).click();
    await expect(page.getByText(/Git facts captured by host/)).toBeVisible();
    await page.getByRole('button', { name: 'Inspect captured diff' }).click();
    await expect(
      page.locator('.intent-packet').filter({ hasText: '+Filtered export preserves permissions' }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Record my verification' }).click();
    await page
      .getByLabel('Evidence or verification notes')
      .fill('I inspected the captured export change and confirmed the permission boundary.');
    await tab(page, 'Intent').click();
    await tab(page, 'Review').click();
    await expect(page.getByLabel('Evidence or verification notes')).toHaveValue(
      'I inspected the captured export change and confirmed the permission boundary.',
    );
    await page.getByRole('button', { name: 'Record evidence', exact: true }).click();
    const verified = page
      .getByRole('article', { name: 'Evidence for Filtered exports preserve permissions' })
      .filter({ hasText: 'Verified by you' });
    await verified.getByRole('checkbox', { name: 'Include in review' }).check();
    await page.getByLabel('Review outcome').selectOption('accept');
    await page
      .getByLabel('Review reason')
      .fill('The selected evidence supports the saved criterion.');
    await page.getByRole('button', { name: 'Record review decision' }).click();
    await expect(page.getByText('Review accepted', { exact: true })).toBeVisible();

    await tab(page, 'Sources').click();
    await page.getByLabel('Source URL', { exact: true }).fill('https://example.invalid/tickets/42');
    await page.getByLabel('Source title').fill('Export requirement');
    await page.getByLabel('Source snapshot').fill('Preserve filtered rows and permissions.');
    await page.getByRole('button', { name: 'Import source' }).click();
    await expect(
      page.getByRole('heading', { name: 'Export requirement', exact: true }),
    ).toBeVisible();
    expect(
      (await host.request({ action: 'sources', id: workspace.id })).sources[0].accepted.content,
    ).toBe('Preserve filtered rows and permissions.');

    await tab(page, 'Knowledge').click();
    await page.getByRole('button', { name: 'Capture document version' }).click();
    await expect(page.getByText(/Captured version is current/)).toBeVisible();
    await page.getByLabel('Finding title').fill('Exports share the authorization boundary');
    await page
      .getByLabel('What should future work know?')
      .fill('Reuse the existing permission checks when adding new export formats.');
    await page.getByRole('checkbox', { name: /\.rivet\/context\/modules\/sample\.md/ }).check();
    await page.getByRole('button', { name: 'Record feature finding' }).click();
    await page.getByRole('button', { name: 'Prepare promotion for review' }).click();
    await expect(page.getByText('Prepared · No project file changed')).toBeVisible();
    const proposal = (await host.request({ action: 'knowledge', id: workspace.id })).proposals[0];
    expect(fs.existsSync(path.join(repo, proposal.path))).toBe(false);
    await page.getByRole('button', { name: 'Write reviewed learning file' }).click();
    await expect(page.getByText('Reviewed content was written')).toBeVisible();
    expect(fs.readFileSync(path.join(repo, proposal.path), 'utf8')).toBe(proposal.content);

    await tab(page, 'Artifacts').click();
    await page.getByText('Add an artifact or version', { exact: true }).click();
    await page.getByLabel('Artifact title').fill('Export preview');
    await page.getByLabel('Upload file (up to 512 KiB)').setInputFiles({
      name: 'preview.png',
      mimeType: 'image/png',
      buffer: Buffer.from(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP6sAAAAASUVORK5CYII=',
        'base64',
      ),
    });
    await page.getByRole('button', { name: 'Save artifact version' }).click();
    await expect(page.getByRole('img', { name: 'Export preview' })).toBeVisible();
    await page.getByRole('button', { name: 'Annotate', exact: true }).click();
    await page
      .getByLabel('Annotation note')
      .fill('Keep the permission explanation beside the export action.');
    await page.getByRole('button', { name: 'Save annotation' }).click();
    await expect(
      page
        .getByText('Keep the permission explanation beside the export action.', { exact: false })
        .last(),
    ).toBeVisible();
    await page.getByText('Create a demonstration', { exact: true }).click();
    await page.getByLabel('Demonstration title').fill('Export walkthrough');
    await page.getByRole('button', { name: 'Add selected screenshot as next step' }).click();
    await page.getByLabel('Step 1 caption').fill('Choose the filtered export action');
    await page.getByRole('button', { name: 'Save demonstration' }).click();
    await expect(page.getByText('Export walkthrough · demonstration · intent 2')).toBeVisible();

    await page.getByText('Compare alternatives', { exact: true }).click();
    await page.getByLabel('Comparison title').fill('Export placement');
    await page.getByLabel('Purpose', { exact: true }).fill('Choose the clearest action placement');
    await page.getByLabel('Alternative 1 name').fill('Beside filters');
    await page
      .getByLabel('Alternative 1 hypothesis')
      .fill('Keeps current filtering context visible');
    await page.getByLabel('Alternative 2 name').fill('Global toolbar');
    await page.getByLabel('Alternative 2 hypothesis').fill('Consistent placement across all pages');
    await page.getByRole('button', { name: 'Save comparison' }).click();
    await page.getByText('Export placement · alternatives · intent 2').click();
    await page.getByLabel('Choose alternative').selectOption({ label: 'Beside filters' });
    await page
      .getByLabel('Selection reason')
      .fill('The export should remain close to its filter context.');
    await page.getByRole('button', { name: 'Record selection' }).click();
    await expect(page.getByRole('heading', { name: 'Beside filters · selected' })).toBeVisible();

    await tab(page, 'Execution').click();
    await page.getByLabel('Control target').selectOption(execution.id);
    await page
      .getByLabel('Reason for interrupting (recorded here)')
      .fill('Pause for a human review');
    await page.getByRole('button', { name: 'Save control for review' }).click();
    expect(callbacks).toHaveLength(0);
    await page.getByRole('button', { name: 'Send interrupt request' }).click();
    await expect(page.getByText('Isolated test transport accepted the request')).toBeVisible();
    expect(callbacks).toHaveLength(1);
    expect(callbacks[0]).toMatchObject({
      method: 'intent.interrupt',
      params: { sessionId, hub: '' },
    });

    await tab(page, 'Overview').click();
    await expect(page.getByText('Your review: Accepted this intent revision')).toBeVisible();
    await expect(page.getByText('Your selection: Beside filters')).toBeVisible();
    await page.evaluate(
      (sessionId) =>
        (window as any).firstUse.update(sessionId, {
          ambientState: 'waiting_approval',
          pendingApproval: {
            toolName: 'Bash',
            toolInput: { command: 'npm test' },
            timestamp: Date.now(),
          },
        }),
      sessionId,
    );
    await expect(page.getByText('Approval needed: Bash', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Open agent to respond' }).click();
    await expect(page.getByRole('button', { name: 'Allow', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Allow', exact: true }).click();
    expect(
      await page.evaluate(
        () =>
          (window as any).firstUse.calls.filter((call: any) => call.method === 'claudeApprove')
            .length,
      ),
    ).toBe(1);
    await work(page).click();
    await tab(page, 'Overview').click();
    await page.screenshot({
      path: testInfo.outputPath('intent-completion-desktop.png'),
      fullPage: true,
      animations: 'disabled',
    });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(
        (theme) => (window as any).electronAPI.saveConfig({ ui: { theme } }),
        theme,
      );
      await expect
        .poll(() => page.evaluate(() => getComputedStyle(document.documentElement).colorScheme))
        .toBe(theme);
      await page.screenshot({
        path: testInfo.outputPath(`intent-completion-desktop-${theme}.png`),
        fullPage: true,
        animations: 'disabled',
      });
    }
    await page.setViewportSize({ width: 390, height: 844 });
    for (const name of ['Overview', 'Review', 'Sources', 'Knowledge', 'Artifacts']) {
      await tab(page, name).click();
      expect(
        await page
          .locator('.intent-detail')
          .evaluate((node) => node.scrollWidth <= node.clientWidth),
        `${name} fits mobile detail width`,
      ).toBe(true);
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
        `${name} fits mobile viewport`,
      ).toBe(true);
    }
    await page.screenshot({
      path: testInfo.outputPath('intent-completion-mobile.png'),
      fullPage: true,
      animations: 'disabled',
    });

    await host.stop();
    host = isolatedHost(repo, env, callbacks);
    const [reopened, evidence, artifacts, knowledge, sources, controls] = await Promise.all(
      ['list', 'evidence', 'artifacts', 'knowledge', 'sources', 'controls'].map((action) =>
        host.request({ action, id: workspace.id }),
      ),
    );
    expect(reopened.workspaces[0]).toMatchObject({
      id: workspace.id,
      revision: 2,
      projectId: workspace.projectId,
      repositoryId: workspace.repositoryId,
    });
    expect(evidence.reviews[0].decision).toBe('accept');
    expect(evidence.evidence).toHaveLength(2);
    expect(artifacts.artifacts).toHaveLength(1);
    expect(artifacts.annotations).toHaveLength(1);
    expect(artifacts.demonstrations).toHaveLength(1);
    expect(artifacts.selections).toHaveLength(1);
    expect(knowledge.captures).toHaveLength(1);
    expect(knowledge.proposals[0].status).toBe('written');
    expect(sources.sources).toHaveLength(1);
    expect(controls.controls[0].attempts[0].status).toBe('accepted');
    await tab(page, 'Overview').click();
    await page.getByRole('button', { name: 'Refresh work overview' }).click();
    await expect(page.getByText('Your review: Accepted this intent revision')).toBeVisible();
    expect(requests).toContain('captureEvidence');
    expect(requests).toContain('publishKnowledgePromotion');
    expect(browserErrors).toEqual([]);
    expect(
      await page.evaluate(
        () =>
          (window as any).firstUse.calls.filter((call: any) => call.method === 'spawnClaude')
            .length,
      ),
    ).toBe(1);
  } finally {
    await host.stop();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('keeps the Work shell usable across widths, themes, mobile selection, and keyboard tabs', async ({
  page,
}, testInfo) => {
  test.setTimeout(120000);
  page.setDefaultTimeout(15000);
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'intent-layout-'));
  const home = path.join(root, 'home'),
    repo = path.join(home, 'project');
  fs.mkdirSync(repo, { recursive: true });
  const env: NodeJS.ProcessEnv = {
    PATH: process.env.PATH,
    HOME: home,
    USERPROFILE: home,
    XDG_CONFIG_HOME: path.join(root, 'config'),
    XDG_DATA_HOME: path.join(root, 'data'),
    XDG_CACHE_HOME: path.join(root, 'cache'),
    APPDATA: path.join(root, 'config'),
    CLAUDE_CONFIG_DIR: path.join(home, '.claude'),
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: '/dev/null',
    LANG: 'C.UTF-8',
  };
  const callbacks: any[] = [];
  const host = isolatedHost(repo, env, callbacks);
  const errors: string[] = [];
  try {
    const first = (
      await host.request({
        action: 'create',
        projectRoot: repo,
        fields: {
          title: 'Export filtered rows',
          outcome: 'Export exactly the rows customers chose.',
          constraints: 'Preserve permissions and column headers.',
          successCriteria: 'Only filtered rows are exported\nPermission checks remain in place',
          sourceUrl: '',
          status: 'draft',
        },
      })
    ).workspace;
    const second = (
      await host.request({
        action: 'create',
        projectRoot: repo,
        fields: {
          title: 'Investigate slow exports',
          outcome: 'Understand why large filtered exports take too long.',
          constraints: 'Keep the existing permission boundary.',
          successCriteria: 'Identify the slow operation',
          sourceUrl: '',
          status: 'draft',
        },
      })
    ).workspace;
    page.on('pageerror', (error) => errors.push(error.message));
    await page.route('**/*', (route) => {
      const url = new URL(route.request().url());
      return url.origin === new URL(base).origin || ['data:', 'blob:'].includes(url.protocol)
        ? route.continue()
        : route.abort();
    });
    await page.exposeFunction('intentTestHost', (request: any) => host.request(request));
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto(`${base}?runtime=ready&theme=dark`);
    await page.getByRole('button', { name: "Got it — don't show again" }).click();
    await page.evaluate(async () => {
      const api = (window as any).electronAPI;
      api.intentWorkspaceRequest = (request: any) => (window as any).intentTestHost(request);
      api.usagePacingSchedule = async () => null;
      await api.saveConfig({ ui: { intentWorkspaces: true } });
    });
    await work(page).click();
    const shell = page.getByRole('region', { name: 'Intent workspaces', exact: true });
    const list = shell.getByRole('complementary', { name: 'Project work' });
    await list.getByRole('button', { name: `${first.title} draft`, exact: true }).click();
    const names = [
      'Overview',
      'Intent',
      'Execution',
      'Direction',
      'Review',
      'Sources',
      'Knowledge',
      'Artifacts',
      'History',
    ];
    const themeColors: Record<string, string> = {};
    for (const width of [320, 768, 1280]) {
      await page.setViewportSize({ width, height: width === 320 ? 800 : 1000 });
      for (const theme of ['dark', 'light']) {
        await page.evaluate(
          (theme) => (window as any).electronAPI.saveConfig({ ui: { theme } }),
          theme,
        );
        await expect
          .poll(() => page.evaluate(() => getComputedStyle(document.documentElement).colorScheme))
          .toBe(theme);
        themeColors[theme] = await shell.evaluate((node) => getComputedStyle(node).backgroundColor);
        if (width === 320) {
          const toggle = shell.getByRole('button', { name: 'Work list', exact: true });
          await expect(toggle).toHaveAttribute('aria-expanded', 'false');
          await expect(list).toBeHidden();
          await toggle.click();
          await expect(toggle).toHaveAttribute('aria-expanded', 'true');
          await expect(list).toBeVisible();
          await list.getByRole('button', { name: `${second.title} draft`, exact: true }).click();
          await expect(toggle).toHaveAttribute('aria-expanded', 'false');
          await expect(list).toBeHidden();
          await expect(
            shell.getByRole('heading', { name: second.title, exact: true }),
          ).toBeVisible();
          await expect(shell.locator('.intent-item[aria-current="page"]')).toHaveAttribute(
            'aria-label',
            `${second.title} draft`,
          );
          await toggle.click();
          await list.getByRole('button', { name: `${first.title} draft`, exact: true }).click();
          await expect(
            shell.getByRole('heading', { name: first.title, exact: true }),
          ).toBeVisible();
        } else {
          await expect(list).toBeVisible();
          await expect(shell.getByRole('button', { name: 'Work list', exact: true })).toBeHidden();
        }
        const rail = shell.getByRole('tablist', { name: 'Workspace views' });
        await expect(rail.getByRole('tab')).toHaveCount(names.length);
        const geometry = await rail.evaluate((node) => {
          const box = node.getBoundingClientRect();
          const tabs = Array.from(node.querySelectorAll('[role="tab"]')).map((tab) =>
            tab.getBoundingClientRect(),
          );
          return {
            height: box.height,
            rowSpread:
              Math.max(...tabs.map((tab) => tab.top)) - Math.min(...tabs.map((tab) => tab.top)),
            scrollWidth: node.scrollWidth,
            width: node.clientWidth,
            overflow: getComputedStyle(node).overflowX,
          };
        });
        expect(geometry.rowSpread, `${width}/${theme}: tabs remain on one row`).toBeLessThanOrEqual(
          1,
        );
        expect(['auto', 'scroll']).toContain(geometry.overflow);
        if (width === 320) expect(geometry.scrollWidth).toBeGreaterThan(geometry.width);
        await rail.locator('[aria-selected="true"]').focus();
        for (const [key, name] of [
          ['End', 'History'],
          ['Home', 'Overview'],
          ['ArrowRight', 'Intent'],
          ['ArrowLeft', 'Overview'],
          ['ArrowLeft', 'History'],
          ['ArrowRight', 'Overview'],
        ]) {
          await page.keyboard.press(key);
          const active = rail.getByRole('tab', { name, exact: true });
          await expect(active).toBeFocused();
          await expect(active).toHaveAttribute('aria-selected', 'true');
          await expect(active).toHaveAttribute('tabindex', '0');
          await expect(rail.locator('[tabindex="0"]')).toHaveCount(1);
          await expect(rail.locator('[role="tab"][tabindex="-1"]')).toHaveCount(names.length - 1);
          const panel = shell.getByRole('tabpanel', { name, exact: true });
          await expect(panel).toBeVisible();
          await expect(panel).toHaveAttribute('id', (await active.getAttribute('aria-controls'))!);
          await expect
            .poll(() =>
              active.evaluate((node) => {
                const rail = node.parentElement!.getBoundingClientRect(),
                  box = node.getBoundingClientRect();
                return box.left >= rail.left - 1 && box.right <= rail.right + 1;
              }),
            )
            .toBe(true);
        }
        for (const name of names) {
          await tab(page, name).click();
          await expect(shell.getByRole('tabpanel', { name, exact: true })).toBeVisible();
          expect(
            await shell
              .locator('.intent-detail')
              .evaluate((node) => node.scrollWidth <= node.clientWidth + 1),
            `${width}/${theme}/${name}: detail fits`,
          ).toBe(true);
          expect(
            await page.evaluate(
              () => document.documentElement.scrollWidth <= window.innerWidth + 1,
            ),
            `${width}/${theme}/${name}: viewport fits`,
          ).toBe(true);
        }
        await tab(page, 'Overview').click();
        if (width === 1280 || width === 320)
          await page.screenshot({
            path: testInfo.outputPath(`intent-shell-${width}-${theme}.png`),
            fullPage: true,
            animations: 'disabled',
          });
      }
    }
    expect(themeColors.dark).not.toBe(themeColors.light);
    expect(callbacks).toEqual([]);
    expect(errors).toEqual([]);
    expect(
      await page.evaluate(
        () =>
          (window as any).firstUse.calls.filter((call: any) => call.method === 'spawnClaude')
            .length,
      ),
    ).toBe(0);
    expect((await host.request({ action: 'list' })).workspaces).toHaveLength(2);
  } finally {
    await host.stop();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
