/**
 * E2E test for the /m mobile PWA client (services/hub/cmd/hub/mobile.html).
 *
 * Runs the real hub with a fake capability provider behind it (see
 * fixtures/mobileHub.ts) and drives the client in a phone-sized Chromium:
 * every screen renders from a real snapshot over the real bus, and every action
 * is asserted on the params that reached the provider.
 *
 * No Electron, no claudemon, no Claude auth — the client only ever speaks the
 * bus protocol, so the provider is the only thing worth faking.
 */
import * as fs from 'fs';
import { test, expect, type Page } from '@playwright/test';
import {
  startMobileHub,
  HOST_TOKEN,
  TRIAGE_TOKEN,
  WORKING_FINISHED,
  MULTI_QUESTION_A,
  MULTI_QUESTION_B,
  type MobileHub,
} from './fixtures/mobileHub';

let hub: MobileHub;

const IPHONE = { width: 390, height: 844 };

test.beforeAll(async () => {
  hub = await startMobileHub();
});
test.afterAll(async () => {
  await hub?.stop();
});

/** Open /m with a token and wait for the fleet to paint. */
async function openClient(page: Page, token = HOST_TOKEN) {
  await page.setViewportSize(IPHONE);
  await page.goto(`${hub.url}/m?token=${token}`);
  await expect(page.locator('#title')).toHaveText('Fleet', { timeout: 10000 });
  await expect(page.locator('.agent').first()).toBeVisible({ timeout: 10000 });
}

test.describe('mobile client', () => {
  test.beforeEach(async ({ page }) => {
    // Each test starts from a clean client (fresh page) and a pristine fleet —
    // pushSnapshot mutations must not leak into the tests that follow.
    hub.reset();
    await page.context().clearCookies();
  });

  test('fleet ranks attention first and renders card telemetry', async ({ page }) => {
    await openClient(page);

    // Attention-first ordering: the two blocked agents come before the working
    // one, which comes before review, then idle.
    const names = await page.locator('.agent .nm').allTextContents();
    expect(names.slice(0, 2).sort()).toEqual(['recon', 'rivet']);
    expect(names).toContain('workspacer');
    expect(names).toContain('djtouchette');

    // The working card carries its live tool line and telemetry.
    const working = page.locator('.agent[data-agent="ws1"]');
    await expect(working.locator('.pill')).toHaveText('In flight');
    await expect(working.locator('.state .txt')).toHaveText('Edit(AgentCard.tsx)');
    await expect(working.locator('.tele')).toContainText('fable-5');
    await expect(working.locator('.tele')).toContainText('33%');
    await expect(working.locator('.tele')).toContainText('$49.30');
    await expect(working.locator('.tele')).toContainText('flow: mobile-audit');

    // The status pill summarises the fleet and the recent/resumable rail exists.
    await expect(page.locator('#statusLabel')).toHaveText(/waiting/);
    await expect(page.locator('.rule .t')).toHaveText('RECENT · RESUMABLE');
    await expect(page.locator('.recent')).toHaveCount(2);
  });

  test('fleet filters narrow by state', async ({ page }) => {
    await openClient(page);
    const total = await page.locator('.agent').count();

    await page.locator('#filters button', { hasText: 'In flight' }).click();
    await expect(page.locator('.agent')).toHaveCount(1);
    await expect(page.locator('.agent .pill')).toHaveText('In flight');

    await page.locator('#filters button', { hasText: 'Waiting' }).click();
    const needs = await page.locator('.agent').count();
    expect(needs).toBeGreaterThanOrEqual(2);

    await page.locator('#filters button', { hasText: 'All' }).click();
    await expect(page.locator('.agent')).toHaveCount(total);
  });

  test('approving from the fleet card sends the decision', async ({ page }) => {
    await openClient(page);
    const recon = page.locator('.agent[data-agent="rec"]');
    await expect(recon.locator('.strip .ask')).toContainText('Allow Bash');
    await expect(recon.locator('.strip .code')).toHaveText('pnpm drizzle-kit push --force');

    await recon.locator('.strip .allow').click();
    await expect.poll(() => hub.callsTo('claude.approve').length).toBe(1);
    expect(hub.callsTo('claude.approve')[0].params).toMatchObject({
      sessionId: 'rec',
      decision: 'yes',
    });
  });

  test('a question renders its options inline, not allow/deny', async ({ page }) => {
    await openClient(page);
    const rivet = page.locator('.agent[data-agent="riv"]');
    await expect(rivet.locator('.strip .ask')).toHaveText(
      'Which auth path should the mobile client take?',
    );
    // Allow/Deny would post an answer the agent never asked for.
    await expect(rivet.locator('.strip .allow')).toHaveCount(0);
    await expect(rivet.locator('.strip .opt')).toHaveCount(3);

    await rivet.locator('.strip .opt').nth(1).click();
    await expect.poll(() => hub.callsTo('claude.answer').length).toBe(1);
    expect(hub.callsTo('claude.answer')[0].params).toMatchObject({
      sessionId: 'riv',
      answers: ['2'],
    });
  });

  test('the y / n key hints on the approval buttons actually work', async ({ page }) => {
    await openClient(page);
    // Highest-priority item is recon's approval, so the keys target it.
    await page.locator('body').press('n');
    await expect.poll(() => hub.callsTo('claude.approve').length).toBe(1);
    expect(hub.callsTo('claude.approve')[0].params).toMatchObject({
      sessionId: 'rec',
      decision: 'no',
    });

    // Typing a message must never fire them.
    await page.locator('.agent[data-agent="ws1"] .top').click();
    await page.locator('#msg').fill('yn');
    await expect.poll(() => hub.callsTo('claude.approve').length).toBe(1);
  });

  test('needs-you inbox lists items by priority with working actions', async ({ page }) => {
    await openClient(page);
    await page.locator('nav button[data-tab="inbox"]').click();
    await expect(page.locator('#title')).toHaveText('Waiting');

    // approval (100) outranks question (95), which outranks bigdiff (40).
    const glyphs = await page.locator('.item .glyph').allTextContents();
    expect(glyphs[0]).toBe('!');
    expect(glyphs[1]).toBe('?');
    expect(glyphs).toContain('±');

    // Snooze removes an item for 30 minutes.
    const before = await page.locator('.item').count();
    await page.locator('.item .sq', { hasText: '◷' }).first().click();
    await expect(page.locator('.item')).toHaveCount(before - 1);

    // Deny from the inbox reaches the provider.
    const denies = hub.callsTo('claude.approve').length;
    await page.locator('.item .deny', { hasText: 'Deny' }).first().click();
    await expect.poll(() => hub.callsTo('claude.approve').length).toBe(denies + 1);
    expect(hub.callsTo('claude.approve').at(-1)!.params).toMatchObject({ decision: 'no' });
  });

  test('a finished turn raises a Finished item on the working→idle edge', async ({ page }) => {
    await openClient(page);
    await page.locator('nav button[data-tab="inbox"]').click();
    await expect(page.locator('.item .glyph', { hasText: '✓' })).toHaveCount(0);

    hub.pushSnapshot(WORKING_FINISHED);

    // The card is identified by its ✓ glyph and carries the agent's closing
    // line, which is what you'd actually want to read on a lock screen.
    const done = page.locator('.item', { has: page.locator('.glyph', { hasText: '✓' }) });
    await expect(done.first()).toBeVisible({ timeout: 5000 });
    await expect(done.first().locator('.who')).toHaveText('workspacer');
    await expect(done.first().locator('.txt')).toContainText('attention feed');
    await expect(done.first().locator('.review')).toHaveText('Review diff');
  });

  test('chat renders work cards, flow cards, changed files and the composer', async ({ page }) => {
    await openClient(page);
    await page.locator('.agent[data-agent="ws1"] .top').click();
    await expect(page.locator('#title')).toHaveText('workspacer');

    // Summary pills.
    await expect(page.locator('.pills button', { hasText: 'flow' })).toBeVisible();
    await expect(page.locator('.pills button', { hasText: 'ctx' })).toBeVisible();

    // User bubble + assistant prose.
    await expect(page.locator('.msg.user .b').first()).toContainText(
      'the mobile fleet list feels flat',
    );
    await expect(page.locator('.say').first()).toContainText('Reading the current route');

    // A collapsible work card that expands into its steps.
    const work = page.locator('.work').first();
    await expect(work.locator('.n')).toHaveText('3 steps');
    await expect(work.locator('.step')).toHaveCount(0);
    await work.locator('.wh').click();
    await expect(work.locator('.step')).toHaveCount(3);
    await expect(work.locator('.step .t').first()).toHaveText('read');
    await expect(work.locator('.step .a').nth(1)).toHaveText('useAttentionFeed(');

    // The running workflow, and the per-turn changed-files card.
    await expect(page.locator('.flowcard .nm')).toHaveText('mobile-audit');
    await expect(page.locator('.flowcard .pr')).toHaveText('2/4 agents');
    await expect(page.locator('.files .fh')).toContainText('CHANGED FILES');

    // Streaming indicator with a working cancel.
    await expect(page.locator('.streaming .lbl')).toBeVisible();
    await page.locator('.streaming button').click();
    await expect.poll(() => hub.callsTo('claude.signal').length).toBeGreaterThan(0);
    expect(hub.callsTo('claude.signal').at(-1)!.params).toMatchObject({
      sessionId: 'ws1',
      signal: 'SIGINT',
    });
  });

  test('the more sheet offers Stop behind a confirm, distinct from Interrupt', async ({ page }) => {
    await openClient(page);
    await page.locator('.agent[data-agent="ws1"] .top').click();

    await page.locator('#moreBtn').click();
    const stopRow = page.locator('#sheet .opt', { hasText: 'Stop this session' });
    await expect(stopRow).toBeVisible();
    await expect(stopRow).toHaveClass(/danger/);
    // Interrupt stays a separate, non-destructive row in the same sheet.
    await expect(page.locator('#sheet .opt', { hasText: 'Interrupt this turn' })).toBeVisible();

    // Dismissing the confirm must not fire the signal.
    page.once('dialog', (d) => d.dismiss());
    const before = hub.callsTo('claude.signal').length;
    await stopRow.click();
    await page.waitForTimeout(300);
    expect(hub.callsTo('claude.signal').length).toBe(before);

    // Accepting it does, with SIGTERM (not SIGINT — the distinction that matters).
    await page.locator('#moreBtn').click();
    page.once('dialog', (d) => d.accept());
    await page.locator('#sheet .opt', { hasText: 'Stop this session' }).click();
    await expect.poll(() => hub.callsTo('claude.signal').length).toBeGreaterThan(before);
    expect(hub.callsTo('claude.signal').at(-1)!.params).toMatchObject({
      sessionId: 'ws1',
      signal: 'SIGTERM',
    });
  });

  test('composer sends a message and switches permission mode', async ({ page }) => {
    await openClient(page);
    await page.locator('.agent[data-agent="ws1"] .top').click();

    await page.locator('#msg').fill('ship it');
    await page.locator('#send').click();
    await expect.poll(() => hub.callsTo('agents.sendMessage').length).toBeGreaterThan(0);
    expect(hub.callsTo('agents.sendMessage').at(-1)!.params).toMatchObject({
      sessionId: 'ws1',
      text: 'ship it',
    });
    await expect(page.locator('#msg')).toHaveValue('');

    // The chips reflect the session and open a picker that really switches.
    await expect(page.locator('#chips button').first()).toContainText('fable-5');
    await page.locator('#chips button', { hasText: 'Ask to approve' }).click();
    await expect(page.locator('#sheet')).toBeVisible();
    await page.locator('#sheet .opt', { hasText: 'Plan mode' }).click();
    await expect.poll(() => hub.callsTo('claude.setPermissionMode').length).toBe(1);
    expect(hub.callsTo('claude.setPermissionMode')[0].params).toMatchObject({
      sessionId: 'ws1',
      mode: 'plan',
    });
  });

  test('a photo attaches via files.upload and rides the message as an [Image:] prefix', async ({
    page,
  }) => {
    await openClient(page);
    await page.locator('.agent[data-agent="ws1"] .top').click();
    await expect(page.locator('#title')).toHaveText('workspacer');

    // The + sheet's "Attach photo" opens the real file chooser. A 1×1 PNG is
    // small enough for the client's PNG passthrough, so the name survives.
    const png = Buffer.from(
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
      'base64',
    );
    await page.locator('#moreBtn').click();
    const chooser = page.waitForEvent('filechooser');
    await page.locator('#sheet .opt', { hasText: 'Attach photo' }).click();
    await (await chooser).setFiles({ name: 'shot.png', mimeType: 'image/png', buffer: png });

    // files.upload is HUB-LOCAL (it never reaches the fake provider), so the
    // proof it ran is the chip flipping from "uploading…" to the file name…
    const chip = page.locator('#attach .att');
    await expect(chip).toHaveCount(1);
    await expect(chip).toContainText('shot.png', { timeout: 10000 });

    await page.locator('#msg').fill('see this');
    await page.locator('#send').click();
    await expect.poll(() => hub.callsTo('agents.sendMessage').length).toBeGreaterThan(0);
    const sent = hub.callsTo('agents.sendMessage').at(-1)!.params;
    expect(sent.sessionId).toBe('ws1');
    // …and the sent text carrying a real path in the hub's landing pad,
    // desktop-parity marker form, with the bytes actually on disk.
    const m = /^\[Image: (\S+workspacer-uploads\/[^\]]+\.png)\] see this$/.exec(sent.text);
    expect(m, `text was: ${sent.text}`).toBeTruthy();
    expect(fs.readFileSync(m![1])).toEqual(png);
    // The composer resets fully after a successful send.
    await expect(page.locator('#attach .att')).toHaveCount(0);
    await expect(page.locator('#msg')).toHaveValue('');
  });

  test('chat hides the tab bar, and the fleet working spinner actually rotates', async ({
    page,
  }) => {
    await openClient(page);
    await expect(page.locator('#nav')).toBeVisible();

    // The working card's ring must ROTATE — the old translateY base transform
    // reduced the animation to matrix interpolation with rotate(360°) ≡
    // identity, i.e. a 2px jiggle. A rotated matrix has b = sin(θ) ≠ 0.
    const spin = page.locator('.agent[data-agent="ws1"] .state .spin');
    await expect(spin).toBeVisible();
    let rotated = false;
    for (let i = 0; i < 5 && !rotated; i++) {
      const b = await spin.evaluate((el) => {
        const t = getComputedStyle(el).transform;
        const p = /matrix\(([^)]+)\)/.exec(t);
        return p ? Math.abs(parseFloat(p[1].split(',')[1])) : 0;
      });
      if (b > 0.05) rotated = true;
      await page.waitForTimeout(150);
    }
    expect(rotated, 'spinner transform never showed a rotation component').toBe(true);

    // Entering chat trades the tab bar for transcript space; the header slims
    // and the back button remains the way out.
    await page.locator('.agent[data-agent="ws1"] .top').click();
    await expect(page.locator('#title')).toHaveText('workspacer');
    await expect(page.locator('#nav')).toBeHidden();
    await expect(page.locator('#hdr')).toHaveClass(/chat/);
    await page.locator('#backBtn').click();
    await expect(page.locator('#nav')).toBeVisible();
  });

  test('a pending question docks above the composer and answers', async ({ page }) => {
    await openClient(page);
    await page.locator('.agent[data-agent="riv"] .neutral').click();
    await expect(page.locator('#title')).toHaveText('rivet');
    await expect(page.locator('#askDock')).toContainText('WAITING ON YOU · QUESTION');
    await expect(page.locator('#askDock .opt')).toHaveCount(3);

    const before = hub.callsTo('claude.answer').length;
    await page.locator('#askDock .opt').first().click();
    await expect.poll(() => hub.callsTo('claude.answer').length).toBe(before + 1);
    expect(hub.callsTo('claude.answer').at(-1)!.params).toMatchObject({
      sessionId: 'riv',
      answers: ['1'],
    });
  });

  test('inspector shows flows, subagents and usage windows', async ({ page }) => {
    await openClient(page);
    await page.locator('.agent[data-agent="ws1"] .top').click();
    await page.locator('.pills button', { hasText: 'flow' }).click();
    await expect(page.locator('#title')).toHaveText('Inspector');

    // Flows: the run, its progress, and its phases with per-agent rows.
    const run = page.locator('.run').first();
    await expect(run.locator('.nm')).toHaveText('mobile-audit');
    await expect(run.locator('.cnt')).toHaveText('2/4');
    await run.locator('.rh').click();
    await expect(page.locator('.phase').first()).toHaveText('PHASE 1 · SURVEY');
    await expect(page.locator('.wa')).toHaveCount(4);
    await expect(page.locator('.wa .nm').first()).toHaveText('route-inventory');
    await expect(page.locator('.wa .line').nth(2)).toContainText('Edit components/AgentCard.tsx');

    // A workflow agent opens its detail sheet.
    await page.locator('.wa').first().click();
    await expect(page.locator('#sheet')).toContainText('tokens');
    await page.locator('#scrim').click();

    // Agents segment.
    await page.locator('#segments button', { hasText: 'Agents' }).click();
    await expect(page.locator('.subcard')).toHaveCount(2);
    await expect(page.locator('.subcard .nm').first()).toHaveText('fleet-card');

    // Usage segment: context, all three rate-limit windows, tiles, warning.
    await page.locator('#segments button', { hasText: 'Usage' }).click();
    await expect(page.locator('.ucard .ubig .v')).toHaveText('33%');
    // fmtTokens keeps one decimal below 10M — same as the desktop's formatter.
    await expect(page.locator('.ucard .ubig .s')).toHaveText('330k / 1.0M');
    await expect(page.locator('.limit')).toHaveCount(3);
    await expect(page.locator('.limit .l').first()).toHaveText('5-hour window');
    await expect(page.locator('.limit .p').first()).toHaveText('62%');
    await expect(page.locator('.limit .rs').first()).toContainText('resets');
    await expect(page.locator('.warn')).toContainText('Monthly usage is at 78%');
    await expect(page.locator('.tile', { hasText: 'SESSION COST' })).toContainText('$49.30');
  });

  test('spawn offers real directories and providers, and spawns', async ({ page }) => {
    await openClient(page);
    await page.locator('nav button[data-tab="new"]').click();
    await expect(page.locator('#title')).toHaveText('Dispatch');

    // Directories come from config.get, favourites starred first.
    await expect(page.locator('.dirrow[data-dir]')).toHaveCount(4);
    await expect(page.locator('.dirrow[data-dir] .star.on')).toHaveCount(2);

    // providers.checkAll disables what isn't installed.
    await expect(page.locator('.provbtn[data-prov="pi"]')).toBeDisabled();
    await expect(page.locator('.provbtn[data-prov="codex"]')).toBeEnabled();

    // Full access is refused for remote spawns, so it can't be selected.
    await expect(page.locator('.permbtn', { hasText: 'Full access' })).toBeDisabled();
    await page.locator('.permbtn', { hasText: 'Accept edits' }).click();

    // A library prompt rides along as the first message.
    await page.locator('.tagchip', { hasText: 'Standup' }).click();
    await page.locator('.dirrow[data-dir]').nth(1).click();
    await page.locator('#spawnGo').click();

    await expect.poll(() => hub.callsTo('agents.spawn').length).toBe(1);
    expect(hub.callsTo('agents.spawn')[0].params).toMatchObject({
      cwd: '/home/djtouchette/Work/rivet-umbrella/rivet',
      provider: 'claude',
      transport: 'stream',
      permissionMode: 'acceptEdits',
    });
  });

  test('a triage token hides operator-only affordances', async ({ page }) => {
    await openClient(page, TRIAGE_TOKEN);

    // Triage can still triage: the approval strip is live.
    await expect(page.locator('.agent[data-agent="rec"] .allow')).toBeVisible();

    // But it cannot spawn, and the client says why instead of offering a button
    // that would die on tap.
    await page.locator('nav button[data-tab="new"]').click();
    await expect(page.locator('#spawnGo')).toBeDisabled();
    await expect(page.locator('.scopenote').last()).toContainText('triage');
    await expect(page.locator('.scopenote').last()).toContainText('operator scope');

    // Model / permission switching is operator-only too.
    await page.locator('nav button[data-tab="fleet"]').click();
    await page.locator('.agent[data-agent="ws1"] .top').click();
    await expect(page.locator('#chips button').first()).toBeDisabled();
  });

  test('a notification deep-link opens that agent', async ({ page }) => {
    await page.setViewportSize(IPHONE);
    // ?agent= is what a tapped push notification lands on.
    await page.goto(`${hub.url}/m?token=${HOST_TOKEN}&agent=riv`);
    await expect(page.locator('#title')).toHaveText('rivet', { timeout: 10000 });
    await expect(page.locator('#askDock')).toContainText('WAITING ON YOU · QUESTION');
  });

  test('an agent with an empty conversation log does not re-render forever', async ({ page }) => {
    await openClient(page);
    // `dj`'s conv log answers seq 0 / no items, the shape claudemon returns for
    // a session that has yet to take a turn. Reading that as "changed" makes
    // renderChat re-arm its own fetch: a 1 Hz rebuild of the whole screen.
    await page.locator('.agent[data-agent="dj"] .top').click();
    await expect(page.locator('#title')).toHaveText('djtouchette');
    await page.evaluate(() => {
      (document.querySelector('#main .body') as HTMLElement).dataset.probe = 'alive';
    });

    const polls = hub.callsTo('sessions.conversation').length;
    await page.waitForTimeout(2600);

    // It still polls for new turns — but it must not rebuild the DOM to
    // discover nothing arrived.
    expect(hub.callsTo('sessions.conversation').length).toBeGreaterThan(polls);
    await expect(page.locator('#main .body[data-probe="alive"]')).toHaveCount(1);
  });

  test('a pick from a declined question set cannot answer the next one', async ({ page }) => {
    await openClient(page);
    hub.pushSnapshot(MULTI_QUESTION_A);
    await page.locator('.agent[data-agent="mq"] .top').click();
    await expect(page.locator('#askDock .opt')).toHaveCount(4);

    // Answer both questions of set A, then decline instead of submitting.
    await page.locator('#askDock .opt').nth(0).click();
    await page.locator('#askDock .opt').nth(2).click();
    await expect(page.locator('#askDock .opt.on')).toHaveCount(2);
    await page.locator('#askDock [data-decline]').click();
    await expect.poll(() => hub.callsTo('claude.signal').length).toBeGreaterThan(0);

    // A different question set arrives. Nothing may be pre-selected...
    hub.pushSnapshot(MULTI_QUESTION_B);
    await expect(page.locator('#askDock')).toContainText('Set B');
    await expect(page.locator('#askDock .opt.on')).toHaveCount(0);

    // ...and a half-filled set must refuse to submit rather than posting a
    // leftover pick as an answer you never chose.
    const answers = hub.callsTo('claude.answer').length;
    await page.locator('#askDock .opt').nth(0).click();
    await page.locator('#askDock [data-submit]').click();
    await expect(page.locator('#toast')).toContainText('Pick an option for every question');
    expect(hub.callsTo('claude.answer').length).toBe(answers);
  });

  test('the token gate appears with no token and connects', async ({ page }) => {
    await page.setViewportSize(IPHONE);
    await page.goto(`${hub.url}/m`);
    await expect(page.locator('.gate h2')).toHaveText('Connect');
    await page.locator('#tok').fill(HOST_TOKEN);
    await page.locator('#tokGo').click();
    await expect(page.locator('.agent').first()).toBeVisible({ timeout: 10000 });
  });

  test('the app shell fills the viewport exactly — no dead band at the bottom', async ({
    page,
  }) => {
    await openClient(page);
    // #app height must come from the MEASURED viewport (--vh), not 100dvh:
    // standalone PWAs can report a dvh that still reserves browser-chrome
    // space, which showed as a toolbar-sized gap under the tab bar.
    const gap = await page.evaluate(() => {
      const app = document.getElementById('app')!.getBoundingClientRect();
      return {
        bottom: Math.abs(window.innerHeight - app.bottom),
        vh: getComputedStyle(document.documentElement).getPropertyValue('--vh').trim(),
      };
    });
    expect(gap.vh).toBe('844px');
    expect(gap.bottom).toBeLessThanOrEqual(1);
  });

  test('screens have no horizontal overflow at phone width', async ({ page }) => {
    await openClient(page);
    const overflow = async () =>
      page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
    expect(await overflow()).toBeLessThanOrEqual(0);

    for (const tab of ['inbox', 'new']) {
      await page.locator(`nav button[data-tab="${tab}"]`).click();
      expect(await overflow()).toBeLessThanOrEqual(0);
    }
    await page.locator('nav button[data-tab="fleet"]').click();
    await page.locator('.agent[data-agent="ws1"] .top').click();
    expect(await overflow()).toBeLessThanOrEqual(0);
  });
});
