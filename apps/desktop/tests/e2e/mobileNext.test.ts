/**
 * E2E for /m-next, the native-aligned phone client
 * (services/hub-rs/assets/web/m-next).
 *
 * The real hub serves the client and routes every call; a fake capability
 * provider (fixtures/mobileNextHub.ts) answers with sparse Rust-hub-shaped
 * rows, so conversations, subagent transcripts and task logs come from their
 * own methods. Each action is asserted on the params that reached the
 * provider. /m is untouched and keeps its own suite (mobileClient.test.ts).
 */
import * as fs from 'fs';
import * as path from 'path';
import { test, expect, type Page } from '@playwright/test';
import { startNextHub, HOST_TOKEN, VIEW_TOKEN, type NextHub } from './fixtures/mobileNextHub';

let hub: NextHub;
const IPHONE = { width: 390, height: 844 };
const SHOTS = process.env.WKS_MNEXT_SHOTS || '';

test.beforeAll(async () => {
  hub = await startNextHub();
});
test.afterAll(async () => {
  await hub?.stop();
});

async function open(
  page: Page,
  opts: { token?: string; scheme?: 'dark' | 'light'; hash?: string } = {},
) {
  await page.emulateMedia({ colorScheme: opts.scheme ?? 'dark', reducedMotion: 'reduce' });
  await page.setViewportSize(IPHONE);
  await page.goto(`${hub.url}/m-next/?token=${opts.token ?? HOST_TOKEN}${opts.hash ?? ''}`);
  if (!opts.hash) await expect(page.locator('.row').first()).toBeVisible({ timeout: 15000 });
}
const row = (page: Page, title: string) =>
  page.locator('.row', { has: page.locator('.title', { hasText: title }) });

test.describe('/m-next', () => {
  const errors: string[] = [];
  test.beforeEach(async ({ page }) => {
    await hub.reset();
    errors.length = 0;
    page.on('pageerror', (e) => errors.push(String(e)));
    page.on('console', (m) => {
      if (
        m.type() === 'error' &&
        !/Failed to load resource|service worker|ServiceWorker/i.test(m.text())
      )
        errors.push(m.text());
    });
  });
  test.afterEach(() => {
    expect(errors, 'page errors').toEqual([]);
  });

  test('is served by the hub as modules with generated tokens and its own fonts', async ({
    request,
  }) => {
    const redirect = await request.get(`${hub.url}/m-next?token=x`, { maxRedirects: 0 });
    expect(redirect.status()).toBe(308);
    expect(redirect.headers()['location']).toBe('/m-next/?token=x');
    const index = await request.get(`${hub.url}/m-next/`);
    expect(index.status()).toBe(200);
    expect(index.headers()['content-security-policy']).toContain("script-src 'self'");
    expect(await index.text()).toContain('type="module" src="./js/main.js"');
    const tokens = await request.get(`${hub.url}/m-next/tokens.css`);
    expect(await tokens.text()).toContain('GENERATED from apps/native/src/appearance.rs');
    const font = await request.get(`${hub.url}/m-next/fonts/Inter-Variable.woff2`);
    expect(font.headers()['content-type']).toBe('font/woff2');
    expect(font.headers()['cache-control']).toContain('immutable');
    const etag = (await request.get(`${hub.url}/m-next/js/main.js`)).headers()['etag'];
    const again = await request.get(`${hub.url}/m-next/js/main.js`, {
      headers: { 'if-none-match': etag },
    });
    expect(again.status()).toBe(304);
    expect((await request.get(`${hub.url}/m-next/../web.rs`)).status()).toBe(404);
    // /m is still the old client.
    expect(await (await request.get(`${hub.url}/m`)).text()).toContain('IBM Plex Sans');
  });

  test('lists sessions with nested children, the Needs-you filter and usage', async ({ page }) => {
    await open(page);
    await expect(row(page, 'Rewrite the getting-started guide').locator('.st')).toHaveText(
      'Working',
    );
    await expect(row(page, 'Retire the v1 ingest path').locator('.st')).toHaveText(
      'Needs approval',
    );
    await expect(row(page, 'Reconcile the fee rounding').locator('.st')).toHaveText(
      'Needs your input',
    );
    // The child session and the provider subagent nest under their parent.
    await expect(row(page, 'Screenshot the new flow')).toHaveClass(/d1/);
    await expect(row(page, 'Explore: find stale links')).toHaveClass(/child/);
    // The host kept `paused` open at close: it is Paused (and cold), not Ended.
    await expect(row(page, 'Migrate settings schema').locator('.st')).toHaveText('Paused');
    await expect(row(page, 'Migrate settings schema').locator('.cold')).toBeVisible();
    await expect(row(page, 'Fix the VAT export').locator('.st')).toHaveText('Ended');
    expect(hub.callsTo('sessions.snapshot').map((c) => c.params.sessionId)).toContain('paused');
    await expect(page.locator('.usage .u')).toHaveCount(2);

    await page.getByRole('tab', { name: /Needs you/ }).click();
    const titles = await page.locator('.row .title').allTextContents();
    expect(titles.sort()).toEqual(['Reconcile the fee rounding', 'Retire the v1 ingest path']);
    await page.getByRole('tab', { name: /Paused/ }).click();
    await expect(page.locator('.row .title')).toHaveText(['Migrate settings schema']);
    if (SHOTS) {
      await page.getByRole('tab', { name: /All/ }).click();
    }
  });

  test('opens a chat with the transcript, tool rows, a response card, a table and highlighted code', async ({
    page,
  }) => {
    await open(page);
    await row(page, 'Rewrite the getting-started guide').click();
    await expect(page.locator('.island .tt')).toHaveText('Rewrite the getting-started guide');
    await expect(page.locator('.you .tx').first()).toContainText('The quickstart still says');
    await expect(page.locator('.prose table th').first()).toHaveText('Command');
    await expect(page.locator('.codeblock .hd span')).toHaveText('rust');
    await expect(page.locator('.codeblock .kw').first()).toHaveText('fn');
    // The response card renders sanitized: no script ran, no image loaded.
    const card = page.locator('.rcard');
    await expect(card.locator('.t')).toHaveText('Docs check');
    await expect(card.locator('table td').first()).toHaveText('install.sh');
    expect(await page.evaluate(() => (window as any).__pwned)).toBeUndefined();
    await expect(card.locator('img, script')).toHaveCount(0);
    // A response-card Prefill fills the composer and never sends.
    await card.getByRole('button', { name: 'Prefill: Publish' }).click();
    await expect(page.locator('[data-input]')).toHaveValue('Publish the new quickstart.');
    expect(hub.callsTo('agents.sendMessage')).toHaveLength(0);
    // Work row expands into tool rows.
    await page.locator('.workrow').first().click();
    await expect(page.locator('.toolrow .v').first()).toHaveText('Edit');
    // Spawn-agent card links its live child.
    await expect(page.locator('.toolcard .childrow')).toContainText('Explore: find stale links');
    await expect(page.locator('.dockline')).toContainText('Working');
  });

  test('sends a message; while working the send queues and Stop interrupts', async ({ page }) => {
    await open(page);
    await row(page, 'Benchmark cold start').click();
    await page.locator('[data-input]').fill('Run it again on the fly node');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await expect.poll(() => hub.callsTo('agents.sendMessage').length).toBe(1);
    expect(hub.callsTo('agents.sendMessage')[0].params).toEqual({
      sessionId: 'ready',
      text: 'Run it again on the fly node',
    });
    await expect(page.locator('.you.pending .meta')).toHaveText('Sent');
    await expect(page.locator('[data-input]')).toHaveValue('');

    await page.locator('[data-back]').click();
    await row(page, 'Rewrite the getting-started guide').click();
    await page.getByRole('button', { name: 'Stop' }).click();
    await expect
      .poll(() => hub.callsTo('claude.signal').map((c) => c.params))
      .toContainEqual({ sessionId: 'work', signal: 'SIGINT' });
  });

  test('answers a two-question set one page at a time', async ({ page }) => {
    await open(page);
    await row(page, 'Reconcile the fee rounding').click();
    const ask = page.getByRole('region', { name: 'Needs your input' });
    await expect(ask.locator('.q')).toContainText('138 already-issued invoices');
    await expect(ask.getByRole('button', { name: /Next/ })).toBeDisabled();
    await ask.locator('.opt', { hasText: 'Reissue corrected invoices' }).click();
    await ask.getByRole('button', { name: /Next/ }).click();
    await expect(ask.locator('.q')).toHaveText('Should customers be told about the change?');
    await ask.getByLabel('Or type a different answer').fill('Only the large accounts');
    await ask.getByRole('button', { name: 'Send answers' }).click();
    await expect.poll(() => hub.callsTo('claude.answer').length).toBe(1);
    expect(hub.callsTo('claude.answer')[0].params).toMatchObject({
      sessionId: 'ques',
      answers: ['Reissue corrected invoices', 'Only the large accounts'],
    });
  });

  test('approves a tool with Allow once', async ({ page }) => {
    await open(page);
    await row(page, 'Retire the v1 ingest path').click();
    const card = page.getByRole('region', { name: 'Needs approval' });
    await expect(card.locator('.tn')).toHaveText('Bash');
    await expect(card.locator('.td')).toHaveText('Drop the v1 ingest tables in staging');
    await expect(card.locator('pre').first()).toContainText('0042_drop_v1_ingest.sql');
    await card.getByRole('button', { name: 'Allow once' }).click();
    await expect
      .poll(() => hub.callsTo('claude.approve').map((c) => c.params))
      .toEqual([{ sessionId: 'appr', decision: 'yes' }]);
  });

  test('resumes a paused session by sending, on its model and access', async ({ page }) => {
    await open(page);
    await row(page, 'Migrate settings schema').click();
    await expect(page.locator('.note .nt').first()).toHaveText('Paused when the app closed');
    await expect(page.locator('.dockline')).toContainText('sending resumes this session');
    await page.locator('[data-input]').fill('Continue the backfill and report every 5,000 rows');
    await page.getByRole('button', { name: 'Resume and send' }).click();
    await expect.poll(() => hub.callsTo('agents.spawn').length).toBe(1);
    expect(hub.callsTo('agents.spawn')[0].params).toMatchObject({
      provider: 'claude',
      cwd: '/workspaces/ledger',
      transport: 'stream',
      resumeSessionId: 'paused',
      model: 'claude-opus-4-8',
      permissionMode: 'default',
      skipPermissions: false,
      message: 'Continue the backfill and report every 5,000 rows',
    });
  });

  test('starts fresh from a summary when the cache has gone cold', async ({ page }) => {
    await open(page);
    await row(page, 'Migrate settings schema').click();
    const cold = page.locator('[data-cold]');
    await expect(cold.locator('.nt')).toHaveText('Cache expired 2h ago');
    await expect(cold.locator('.nb')).toContainText('~624K tokens (≈$6.24, vs $0.31 warm)');
    await cold.getByRole('button', { name: 'Start fresh from a summary' }).click();
    const sheet = page.getByRole('dialog', { name: 'Handoff' });
    await expect(sheet.locator('h3')).toHaveText('Start fresh from a summary');
    await expect(sheet.getByRole('button', { name: 'Summary by a fast model' })).toHaveClass(/on/);
    // An ended agent cannot write its own brief.
    await expect(sheet.getByRole('button', { name: 'Written by Claude' })).toBeDisabled();
    await sheet.locator('[data-go]').click();
    await expect.poll(() => hub.callsTo('claude.handoffSummaryBrief').length).toBe(1);
    expect(hub.callsTo('claude.handoffSummaryBrief')[0].params).toEqual({ sessionId: 'paused' });
    await expect.poll(() => hub.callsTo('agents.spawn').length).toBe(1);
    const spawn = hub.callsTo('agents.spawn')[0].params;
    expect(spawn).toMatchObject({
      provider: 'claude',
      cwd: '/workspaces/ledger',
      transport: 'stream',
      permissionMode: 'default',
    });
    expect(spawn.message).toBeUndefined();
    expect(spawn.resumeSessionId).toBeUndefined();
    // The successor opens with the takeover message staged, not sent.
    await expect(page.locator('[data-input]')).toHaveValue(
      /read the handoff brief at \/workspaces\/ledger\/\.workspacer\/handoffs\/2026-10-08-migrate\.md/,
    );
    expect(hub.callsTo('agents.sendMessage')).toHaveLength(0);
  });

  test('shows a background task log that follows, and stops it after confirming', async ({
    page,
  }) => {
    await open(page);
    await row(page, 'Rewrite the getting-started guide').click();
    await page.getByRole('button', { name: '2 background tasks' }).click();
    const sheet = page.getByRole('dialog', { name: 'Background tasks' });
    await expect(sheet.locator('.badge')).toHaveText('2 running');
    await sheet.locator('.tk', { hasText: 'npm run docs:dev' }).click();
    const log = sheet.locator('[data-log]');
    await expect(log).toContainText('ready in 412 ms');
    // It follows: later reads append from the last offset.
    await expect(log).toContainText('built install in 61 ms', { timeout: 5000 });
    const offsets = hub.callsTo('sessions.taskOutput').map((c) => c.params.offset);
    expect(offsets[0]).toBeUndefined();
    expect(offsets.slice(1).every((o) => typeof o === 'number' && o > 0)).toBe(true);
    await expect(sheet.getByRole('button', { name: 'Following' })).toBeVisible();
    await sheet.getByRole('button', { name: 'Stop task' }).click();
    await page
      .getByRole('dialog', { name: 'Stop this task?' })
      .getByRole('button', { name: 'Stop task' })
      .click();
    await expect
      .poll(() => hub.callsTo('sessions.taskStop').map((c) => c.params))
      .toEqual([{ sessionId: 'work', taskId: 'task-shell' }]);
  });

  test('opens a provider subagent read-only, with its parent one tap away', async ({ page }) => {
    await open(page);
    await row(page, 'Explore: find stale links').click();
    await expect(page.locator('.island .tt')).toContainText('Explore: find stale links');
    await expect(page.locator('.thread .prose').last()).toContainText(
      'Three links still point at the old wiki',
    );
    expect(hub.callsTo('sessions.subagentConversation')[0].params).toEqual({
      sessionId: 'work',
      agentId: 'sub-links',
    });
    await expect(page.locator('[data-input]')).toHaveCount(0);
    await page.getByRole('button', { name: 'Open parent' }).click();
    await expect(page.locator('.island .tt')).toHaveText('Rewrite the getting-started guide');
  });

  test('the session menu switches access and archives', async ({ page }) => {
    await open(page);
    await row(page, 'Benchmark cold start').click();
    await page.getByRole('button', { name: 'Session menu' }).click();
    const menu = page.getByRole('dialog', { name: 'Session menu' });
    await menu.getByRole('button', { name: /Access/ }).click();
    await page
      .getByRole('dialog', { name: 'Access' })
      .getByRole('button', { name: /Accept edits/ })
      .click();
    await expect
      .poll(() => hub.callsTo('claude.setPermissionMode').map((c) => c.params))
      .toEqual([{ sessionId: 'ready', mode: 'acceptEdits' }]);
    await page.getByRole('button', { name: 'Session menu' }).click();
    await page
      .getByRole('dialog', { name: 'Session menu' })
      .getByRole('button', { name: 'Archive' })
      .click();
    await page.locator('[data-back]').click();
    await expect(row(page, 'Benchmark cold start')).toHaveCount(0);
  });

  test('approves a proposed job and pins a project', async ({ page }) => {
    await open(page);
    await page.getByRole('button', { name: 'Jobs', exact: true }).click();
    const job = page.locator('.job', { hasText: 'Nightly dependency check' });
    await expect(job.locator('.badge')).toHaveText('Proposed');
    await expect(job).toContainText('Check for outdated dependencies');
    await job.getByRole('button', { name: 'Approve' }).click();
    await expect(job.locator('.badge')).toHaveText('On', { timeout: 10000 });

    await page.locator('[data-back]').click();
    await page.getByRole('button', { name: 'Projects', exact: true }).click();
    await page.getByRole('button', { name: 'Pin ledger' }).click();
    await expect(page.getByRole('button', { name: 'Unpin ledger' })).toBeVisible();
    const save = hub.callsTo('config.save').at(-1)!.params;
    expect(save.projects['/workspaces/ledger'].favourite).toBe(true);
    expect(save.projects['/workspaces/orbital'].favourite).toBe(true);
  });

  test('switches theme, and follows the phone by default', async ({ page }) => {
    await open(page, { scheme: 'light' });
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('radio', { name: 'Nord' }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'nord');
    const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    expect(bg).toBe('rgb(37, 42, 51)'); // native Nord `chat` #252a33
    await page.getByRole('switch', { name: 'Match the phone' }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  });

  test('starts a new agent from the floating button', async ({ page }) => {
    await open(page);
    await page.getByRole('button', { name: 'New agent' }).click();
    await expect(page.locator('.proj .pn')).toContainText('orbital');
    await expect(page.locator('.gitline')).toContainText('docs/installer · 2 uncommitted changes');
    await page.getByRole('button', { name: /Codex/ }).click();
    await page.getByLabel('Model').selectOption({ label: 'GPT-5.6 Luna' });
    await page.getByLabel('Task').fill('Screenshot the quickstart at phone size');
    await page.getByRole('button', { name: /Start agent/ }).click();
    await expect.poll(() => hub.callsTo('agents.spawn').length).toBe(1);
    expect(hub.callsTo('agents.spawn')[0].params).toMatchObject({
      provider: 'codex',
      cwd: '/workspaces/orbital',
      transport: 'stream',
      model: 'gpt-5.6-luna',
      permissionMode: 'ask',
      skipPermissions: false,
      message: 'Screenshot the quickstart at phone size',
      autoTitle: true,
    });
    await expect(page.locator('.chattop')).toBeVisible();
  });

  test('swiping a row reveals Archive, and a finished subagent can be cleared', async ({
    page,
  }) => {
    await open(page);
    const swipe = async (title: string) => {
      const box = (await row(page, title).boundingBox())!;
      await page.mouse.move(box.x + box.width - 40, box.y + box.height / 2);
      await page.mouse.down();
      for (let i = 1; i <= 8; i++)
        await page.mouse.move(box.x + box.width - 40 - i * 25, box.y + box.height / 2);
      await page.mouse.up();
    };
    await swipe('Fix the VAT export');
    await page.getByRole('button', { name: 'Archive', exact: true }).click();
    await expect.poll(() => hub.callsTo('sessionArchive.set').length).toBe(0); // hub-owned, not the provider
    await expect(row(page, 'Fix the VAT export')).toHaveCount(0);
    await page.getByRole('button', { name: 'History', exact: true }).click();
    await page.getByRole('tab', { name: 'Archived' }).click();
    await expect(page.locator('.hrow .n')).toContainText(['Fix the VAT export']);
    await page
      .locator('.hrow', { hasText: 'Fix the VAT export' })
      .getByRole('button', { name: 'Restore' })
      .click();
    await page.locator('[data-back]').click();
    await expect(row(page, 'Fix the VAT export')).toHaveCount(1);
    // The subagent finishes: it stays listed until cleared (device-local).
    const work = hub.snapshots.get('work');
    hub.pushSnapshot({
      ...work,
      subagents: [{ ...work.subagents[0], status: 'complete', completedAt: Date.now() }],
    });
    await expect(row(page, 'Explore: find stale links').locator('.st')).toHaveText('Completed');
    await swipe('Explore: find stale links');
    await page.getByRole('button', { name: 'Clear', exact: true }).click();
    await expect(row(page, 'Explore: find stale links')).toHaveCount(0);
  });

  test('a view token reads but cannot act', async ({ page }) => {
    await open(page, { token: VIEW_TOKEN });
    await row(page, 'Retire the v1 ingest path').click();
    const card = page.getByRole('region', { name: 'Needs approval' });
    await expect(card.getByRole('button', { name: 'Allow once' })).toBeDisabled();
    await expect(page.locator('[data-input]')).toBeDisabled();
  });
});

// ── screenshots (WKS_MNEXT_SHOTS=<dir>): the real client at 390×844 ───────
test.describe('/m-next screenshots', () => {
  test.skip(!SHOTS, 'set WKS_MNEXT_SHOTS to a directory to capture');
  // Same density as the phase-1 mockups (390×844 @2x).
  test.use({ deviceScaleFactor: 2 });
  test.beforeEach(() => hub.reset());
  for (const scheme of ['dark', 'light'] as const) {
    test(`captures every screen (${scheme})`, async ({ page }) => {
      test.setTimeout(120_000);
      fs.mkdirSync(SHOTS, { recursive: true });
      await page.setViewportSize(IPHONE);
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
      const shot = async (name: string) => {
        await page.waitForTimeout(350);
        await page.screenshot({ path: path.join(SHOTS, `${name}-${scheme}.png`) });
      };
      const go = async (hash: string) => {
        await page.goto(`${hub.url}/m-next/?token=${HOST_TOKEN}${hash}`);
        await page.waitForTimeout(900);
      };
      await go('#/');
      await expect(page.locator('.row').first()).toBeVisible();
      await shot('sessions');
      await go('#/s/work');
      await expect(page.locator('.rcard')).toBeVisible();
      await shot('chat');
      await page.getByRole('button', { name: 'Session menu' }).click();
      await shot('island');
      await page.keyboard.press('Escape');
      await go('#/s/work');
      await page.getByRole('button', { name: '2 background tasks' }).click();
      await page.locator('.tk', { hasText: 'npm run docs:dev' }).click();
      await page.waitForTimeout(1500);
      await shot('tasks');
      await go('#/s/ques');
      await shot('question');
      await go('#/s/appr');
      await shot('approval');
      await go('#/s/paused');
      await shot('paused');
      await page.locator('[data-cold] [data-fresh]').click();
      await shot('handoff');
      await go('#/s/work/a/sub-links');
      await shot('child');
      await go('#/new');
      await shot('new');
      await go('#/history');
      await shot('history');
      await go('#/settings');
      await shot('settings');
      await go('#/jobs');
      await shot('jobs');
      await go('#/projects');
      await shot('projects');
    });
  }
});
