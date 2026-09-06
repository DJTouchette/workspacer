/**
 * Response cards, in a REAL browser.
 *
 * Everything asserted here is something jsdom cannot answer: whether Chromium
 * enforces the shell's Content-Security-Policy, whether the iframe sandbox
 * actually withholds the parent document, whether a card can navigate itself or
 * fetch anything, and whether the frame is a keyboard trap.
 *
 * The page under test is `html-card-harness.html`, which renders the PRODUCTION
 * `ConversationMessage` → `parseMarkdownBlocks` → `HtmlResponseCard` path.
 *
 * Method for the negative results: every request the browser issues is recorded
 * on `page.on('request')`, and the harness fires ONE control request of its own
 * at startup. Blocked resources must fail specifically with CSP and never
 * reach request routing; the control proves observation is working.
 */
import { test, expect, type Page, type Request } from '@playwright/test';
import { spawn, type ChildProcess } from 'child_process';
import * as path from 'path';
import { freePort } from './fixtures/scratchState';

let vite: ChildProcess;
let base: string;

test.beforeAll(async ({ browser }) => {
  console.info('HTML-card Chromium version:', browser.version());
  const port = await freePort();
  base = `http://127.0.0.1:${port}/html-card-harness.html`;
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
    {
      cwd: path.resolve(__dirname, '../../src/renderer'),
      stdio: 'ignore',
    },
  );
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(base)).ok) return;
    } catch {
      /* starting */
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('Response-card harness did not start');
});
test.afterAll(() => vite?.kill());
const escapedRoutes = new WeakMap<Page, string[]>();
test.beforeEach(async ({ page }) => {
  escapedRoutes.set(page, []);
  // Observe attempted escapes, but abort them before reaching any real service.
  await page.route('**/*', (route) =>
    new URL(route.request().url()).origin === new URL(base).origin
      ? route.continue()
      : (escapedRoutes.get(page)!.push(route.request().url()), route.abort()),
  );
});

/** Record every request the page tries to make, and block anything leaving the
 *  harness origin so a failure cannot actually exfiltrate during a test run. */
function watchRequests(page: Page): { urls: string[]; failures: Map<string, string> } {
  const urls: string[] = [];
  const failures = new Map<string, string>();
  page.on('request', (r: Request) => urls.push(r.url()));
  page.on('requestfailed', (r: Request) => failures.set(r.url(), r.failure()?.errorText ?? ''));
  return { urls, failures };
}

const OFF_ORIGIN = (u: string) => new URL(u).origin !== new URL(base).origin;

test('a card renders, keeps the prose, and shows its declared actions in app chrome', async ({
  page,
}) => {
  await page.goto(base);
  const card = page.getByTestId('wks-html-card').first();
  await expect(card).toBeVisible();
  await expect(page.getByText('Here is the review.')).toBeVisible();

  // The frame is opaque-origin and srcDoc-only.
  const frame = page.getByTestId('wks-html-card-frame').first();
  await expect(frame).toHaveAttribute('sandbox', 'allow-scripts');
  expect(await frame.getAttribute('src')).toBeNull();

  // The buttons are OUTSIDE the frame — real app DOM the card cannot draw.
  const button = page.getByRole('button', { name: 'Prefill: Draft the reply' });
  await expect(button).toBeVisible();
  expect(await button.evaluate((el) => el.ownerDocument === document)).toBe(true);
});

test('the host runtime gives the card local filter, sort and disclosure', async ({ page }) => {
  await page.goto(base);
  const frame = page.frameLocator('[data-testid="wks-html-card-frame"]').first();
  const rows = frame.locator('#rows tbody tr');
  await expect(rows).toHaveCount(3);

  await frame.getByLabel('Filter findings').fill('auth');
  await expect(frame.locator('#rows tbody tr:visible')).toHaveCount(2);
  await expect(frame.locator('[data-wks-filter-count]')).toHaveText('2');

  await frame.getByLabel('Filter findings').fill('');
  // Sort by the numeric column, then reverse it.
  await frame.locator('th[data-wks-sort="number"]').click();
  await expect(frame.locator('#rows tbody tr').first().locator('td').first()).toHaveText(
    'auth/session.ts',
  );
  await frame.locator('th[data-wks-sort="number"]').click();
  await expect(frame.locator('#rows tbody tr').first().locator('td').first()).toHaveText(
    'db/index.ts',
  );

  // <details> works with no script at all, and the frame regrows for it.
  const before = await page.getByTestId('wks-html-card-frame').first().boundingBox();
  await frame.getByText('Why the first one matters').click();
  await expect
    .poll(async () => (await page.getByTestId('wks-html-card-frame').first().boundingBox())!.height)
    .toBeGreaterThan(before!.height);
});

test('hostile markup reaches the network zero times and navigates nothing', async ({ page }) => {
  const seen = watchRequests(page);
  await page.goto(base);
  await expect(page.getByTestId('wks-html-card').nth(1)).toBeVisible();

  const hostile = page.frameLocator('[data-testid="wks-html-card-frame"]').nth(1);
  // The card's own words survive; only the machinery is gone.
  await expect(hostile.locator('#survivor')).toHaveText('the words survive');

  // Sanitizer: none of the navigating/embedding elements exist at all.
  for (const sel of ['meta', 'form', 'iframe', 'object', 'link[rel=stylesheet]', 'script']) {
    await expect(
      hostile.locator('body ' + sel + (sel === 'script' ? ':not([nonce])' : '')),
      sel,
    ).toHaveCount(0);
  }
  // Anchors survive as text but carry no href, so a click cannot navigate.
  await expect(hostile.locator('a#ext')).toHaveCount(1);
  expect(await hostile.locator('a#ext').getAttribute('href')).toBeNull();
  await hostile.locator('a#ext').click();
  expect(await hostile.locator('a#loop').getAttribute('href')).toBeNull();
  await hostile.locator('a#loop').click();
  await hostile.locator('#clickme').click();

  // Give a meta-refresh / late load every chance to fire.
  await page.waitForTimeout(1200);

  // The frame is still the srcDoc document, not something it navigated to.
  const frames = page.frames().filter((f) => f.url() !== page.url());
  for (const f of frames) expect(f.url(), 'a card frame navigated').not.toContain('exfil.invalid');
  expect(page.url()).toContain('html-card-harness.html');

  // The control request proves the observer works; nothing else left the box.
  expect(seen.urls.some((u) => u.includes('__harness_probe_control'))).toBe(true);
  expect(escapedRoutes.get(page)).toEqual([]);
  for (const url of seen.urls.filter(OFF_ORIGIN))
    expect(seen.failures.get(url), url).toMatch(/^(?:csp|net::ERR_BLOCKED_BY_CSP)$/);
});

test('a script the sanitizer missed still cannot run, fetch, navigate or reach the parent', async ({
  page,
}) => {
  const seen = watchRequests(page);
  await page.goto(`${base}?surface=probe`);
  await page.waitForTimeout(1500);

  const probes = (await page.evaluate(
    () => (window as unknown as { __probes: Array<{ probe?: string }> }).__probes,
  )) as Array<{ probe?: string }>;
  const kinds = probes.map((p) => p?.probe).filter(Boolean);

  // CONTROL: with the host nonce, the very same script runs. Without this the
  // negative below would only mean "the frame never loaded".
  expect(kinds, 'the nonced control script did not run — the experiment is void').toContain(
    'script-ran',
  );
  // …exactly once. The un-nonced twin was refused by CSP.
  expect(kinds.filter((k) => k === 'script-ran').length).toBe(1);

  // Even the nonced script cannot reach the parent document: no
  // allow-same-origin means `parent.document` throws.
  expect(kinds).not.toContain('parent-dom');
  expect(await page.title()).not.toBe('pwned');

  // …and cannot fetch anything, external or loopback (`connect-src 'none'`).
  expect(escapedRoutes.get(page)).toEqual([]);
  for (const url of seen.urls.filter(OFF_ORIGIN))
    expect(seen.failures.get(url), url).toMatch(/^(?:csp|net::ERR_BLOCKED_BY_CSP)$/);
  expect(seen.urls.some((u) => u.includes('__harness_probe_control'))).toBe(true);

  // The nonce-free generated script cannot navigate; the nonced control
  // also cannot navigate the top frame or open a window. CSP does not
  // prevent a trusted script from navigating its own frame.
  expect(page.url()).toContain('html-card-harness.html');
  for (const f of page.frames()) expect(f.url()).not.toContain('exfil.invalid');
});

test('an unknown version shows the fallback prose, a partial fence shows nothing live', async ({
  page,
}) => {
  await page.goto(base);
  await expect(page.getByText('A v99 card.', { exact: true })).toBeVisible();
  await expect(page.getByText(/Card not shown — this card is version 99/)).toBeVisible();
  // Four assistant turns, but only two of them are live cards: the review and
  // the hostile one. The v99 refusal and the still-streaming fence are not.
  await expect(page.getByTestId('wks-html-card')).toHaveCount(2);
});

test('a user message with the same fence is never a card', async ({ page }) => {
  await page.goto(base);
  // The last turn is the pasted one; it renders as text inside the user bubble.
  await expect(
    page.getByText('"v":1,"title":"Review: 3 findings"', { exact: false }).last(),
  ).toBeVisible();
  await expect(page.getByTestId('wks-html-card')).toHaveCount(2);
});

test('the card follows the theme, live, and fits narrow and wide', async ({ page }, info) => {
  for (const width of [360, 1200]) {
    await page.setViewportSize({ width, height: 900 });
    for (const theme of ['dark', 'light']) {
      await page.goto(`${base}?theme=${theme}&v=${width}`);
      const frame = page.getByTestId('wks-html-card-frame').first();
      await expect(frame).toBeVisible();
      // The frame's document carries the app's own resolved tokens, so a card
      // is never a white rectangle in a dark app.
      const srcdoc = (await frame.getAttribute('srcdoc')) ?? '';
      expect(srcdoc).toContain('--wks-text-primary:');
      expect(srcdoc).toContain(`color-scheme: ${theme === 'light' ? 'light' : 'dark'}`);
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
        `horizontal overflow at ${width}px`,
      ).toBe(true);
      await page.screenshot({
        path: info.outputPath(`card-${theme}-${width}.png`),
        fullPage: true,
      });
    }
    // …and switching theme with the card on screen re-themes it in place.
    await page.goto(`${base}?theme=dark&v=${width}`);
    const before = (await page.getByTestId('wks-html-card-frame').first().getAttribute('srcdoc'))!;
    await page.locator('#before-card').click();
    await expect
      .poll(async () => page.getByTestId('wks-html-card-frame').first().getAttribute('srcdoc'))
      .not.toBe(before);
    expect(await page.getByTestId('wks-html-card-frame').first().getAttribute('srcdoc')).toContain(
      'color-scheme: light',
    );
  }
});

test('the card is announced by name and is not a keyboard trap', async ({ page }) => {
  await page.goto(base);
  await expect(page.getByTitle('Response card: Review: 3 findings')).toHaveCount(1);
  await expect(page.getByLabel('Response card: Review: 3 findings')).toHaveCount(1);

  // Tab from before the card: focus must reach the frame…
  await page.locator('#before-card').focus();
  await page.keyboard.press('Tab');
  expect(await page.evaluate(() => document.activeElement?.tagName)).toBe('IFRAME');
  // …and must be able to leave it again. The frame contains a real focusable
  // (the filter box), so this is a genuine trap risk, not a formality.
  for (let i = 0; i < 25; i++) {
    const id = await page.evaluate(() => document.activeElement?.id ?? '');
    if (id === 'after-card') break;
    await page.keyboard.press('Tab');
  }
  expect(await page.evaluate(() => document.activeElement?.id)).toBe('after-card');
});

test('the frame is bounded and scrolls inside itself rather than growing forever', async ({
  page,
}) => {
  await page.goto(base);
  for (const frame of await page.getByTestId('wks-html-card-frame').all()) {
    const box = await frame.boundingBox();
    expect(box!.height).toBeLessThanOrEqual(560);
    expect(box!.height).toBeGreaterThan(0);
  }
});

test('resize messages require the exact frame sender and cannot invoke actions', async ({
  page,
}) => {
  await page.goto(base);
  const first = page.getByTestId('wks-html-card-frame').first();
  await expect(first).toBeVisible();
  await page.waitForTimeout(150);
  const original = await first.evaluate((element) => (element as HTMLElement).style.height);
  await page.evaluate(() => window.postMessage({ type: 'wks-card-height', height: 90000 }, '*'));
  await page.waitForTimeout(100);
  expect(await first.evaluate((element) => (element as HTMLElement).style.height)).toBe(original);
  const child = page
    .frames()
    .find((frame) => frame.parentFrame() && frame.url() === 'about:srcdoc')!;
  await child.evaluate(() => parent.postMessage({ type: 'wks-card-height', height: 90000 }, '*'));
  await expect(first).toHaveCSS('height', '560px');
  await child.evaluate(() => parent.postMessage({ type: 'wks-card-height', height: -100 }, '*'));
  await expect(first).toHaveCSS('height', '40px');
  await page.evaluate(() => {
    (window as unknown as { cardEvents: string[] }).cardEvents = [];
    for (const type of ['library:insert', 'review:request-file', 'watch:session'])
      window.addEventListener(type, () =>
        (window as unknown as { cardEvents: string[] }).cardEvents.push(type),
      );
  });
  await child.evaluate(() => {
    parent.postMessage({ type: 'wks-card-action', kind: 'fill_composer', text: 'attack' }, '*');
    parent.postMessage({ type: 'wks-card-height', height: '100000' }, '*');
  });
  await page.waitForTimeout(100);
  expect(
    await page.evaluate(() => (window as unknown as { cardEvents: string[] }).cardEvents),
  ).toEqual([]);
  await expect(first).toHaveCSS('height', '40px');
});
