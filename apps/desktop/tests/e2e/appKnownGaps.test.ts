/**
 * `/app`'s KNOWN GAPS — the specs that are red on master today, on purpose.
 *
 * Each one is written against the behaviour the user should get, not the
 * behaviour they get now, and each is annotated `test.fail()` with the worker
 * whose in-flight change closes it. So:
 *
 *   • today   — the annotation makes the run GREEN while the assertion fails,
 *               and the annotation itself is the documentation of the gap;
 *   • on fix  — Playwright reports "expected to fail but passed", which is a
 *               loud instruction to delete the annotation line. The spec then
 *               becomes an ordinary regression guard.
 *
 * You can watch a fix land without touching the file: set the env flag named on
 * the annotation (e.g. `WKS_E2E_WEBVIEW_FIXED=1`) and the spec is asserted for
 * real.
 *
 * Nothing here is weakened to pass. The three gaps are the top three findings
 * of `.workspacer/reports/2026-08-24-web-client-completeness.md` §0, in its
 * order:
 *
 *   1. every `<webview>`-backed pane (plugin panes, the Browser pane, the
 *      widget board) paints NOTHING in a browser — the report's number one
 *      blocker, and the reason this whole harness exists: it would have caught
 *      it on day one;
 *   2. a live permission-mode switch fails with nothing but a `console.warn`
 *      (`ComposerControls.tsx:474-477`);
 *   3. attaching a file is a `window.prompt` asking you to type HOST paths
 *      (`webBackend.ts:871-883`), while `/m` has had a working upload for
 *      weeks.
 */
import { test, expect, type Page } from '@playwright/test';
import { startAppHub, layoutOf, workspace, pane, type AppHub } from './fixtures/appHub';

let hub: AppHub;
const SESSION = 'ws1';

test.beforeAll(async () => {
  hub = await startAppHub({
    layout: layoutOf(
      workspace('agent-1', {
        name: 'working',
        sessionId: SESSION,
        panes: [
          pane('claude', {
            title: 'working',
            transport: 'stream',
            attachSessionId: SESSION,
            expectHistory: true,
          }),
        ],
      }),
    ),
  });
});

test.afterAll(async () => {
  await hub?.stop();
});

test.beforeEach(() => hub.reset());

async function openApp(page: Page): Promise<void> {
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto(hub.appUrl, { waitUntil: 'load' });
  await expect(page.getByText('Reading the current route')).toBeVisible({ timeout: 30_000 });
}

// ═══ gap 1 — webview panes paint nothing ════════════════════════════════════

test.describe('panes that load a URL', () => {
  /** Swap the whole layout for one holding a single URL-loading pane, then
   *  reload so it mounts from a clean hydration. */
  async function openPaneOfType(page: Page, type: string, extra: Record<string, unknown> = {}) {
    hub.setLayout(
      layoutOf(
        workspace('url-pane', {
          name: type,
          panes: [pane(type, { title: type, url: hub.contentUrl, ...extra })],
        }),
      ),
    );
    await page.goto(hub.appUrl, { waitUntil: 'load' });
    // The pane chrome paints regardless; give the content a real chance to load.
    await page.waitForTimeout(2_500);
  }

  /**
   * Did the pane actually load our content server's page?
   *
   * Deliberately agnostic about HOW: it looks for any nested browsing context
   * whose URL is the content server and reads the marker out of it. An
   * `<iframe>` is the expected fix, but `<object>`/`<embed>` or a wrapper
   * component would satisfy the user-visible requirement just as well, and a
   * spec that pins the tag would fail a correct implementation.
   */
  async function paneContentLoaded(page: Page): Promise<boolean> {
    const frame = page.frames().find((f) => f.url().startsWith(hub.contentUrl));
    if (!frame) return false;
    try {
      return (await frame.locator('body').innerText()).includes(hub.contentMarker);
    } catch {
      return false; // frame detached mid-read
    }
  }

  test('a plugin pane renders the plugin, not a blank box', async ({ page }) => {
    // ── Expected to fail until web-1's iframe fallback for webview panes lands.
    test.fail(!process.env.WKS_E2E_WEBVIEW_FIXED);

    await openPaneOfType(page, 'plugin', { pluginId: 'demo' });

    // Assert on CONTENT, not on the element. `<webview>` is an unknown element
    // in Chromium, so it exists, has the right `src`, sizes itself normally —
    // and loads nothing. Every check short of reading across the frame boundary
    // passes against a blank pane, which is exactly why this gap survived a
    // parity guard that checks API methods and a DOM that looks fine.
    // 8s is generous for a loopback frame and keeps this failing-by-design case
    // from dominating the suite's runtime.
    await expect
      .poll(() => paneContentLoaded(page), {
        timeout: 8_000,
        message: 'the plugin pane loaded no content — it is a blank box',
      })
      .toBe(true);
  });

  test('the Browser pane is either usable or honest — never a silent blank', async ({ page }) => {
    // ── Expected to fail until web-1 lands. The audit's recommendation is that
    // the ARBITRARY-URL Browser pane may legitimately stay desktop-only on web
    // (`<webview>`'s partition/allowpopups isolation has no iframe equivalent,
    // and X-Frame-Options blocks many sites) — so this accepts either outcome.
    // What it refuses is the current one: a pane that looks fine and shows
    // nothing, with no explanation anywhere on screen.
    test.fail(!process.env.WKS_E2E_WEBVIEW_FIXED);

    await openPaneOfType(page, 'browser');

    if (await paneContentLoaded(page)) return; // rendered for real — fine.

    // Otherwise it must SAY so, in words a user can act on.
    const body = await page.locator('body').innerText();
    expect(
      /desktop|browser pane|not (available|supported)|open (it )?in/i.test(body),
      'the Browser pane rendered nothing and explained nothing',
    ).toBe(true);
  });
});

// ═══ gap 2 — a failed permission-mode switch is silent ══════════════════════

test.describe('failures the user should see', () => {
  test('a rejected permission-mode switch is surfaced, not just console.warn-ed', async ({
    page,
  }) => {
    // ── Expected to fail until web-2 surfaces the rejection.
    test.fail(!process.env.WKS_E2E_LOUD_FAILURES_FIXED);

    await openApp(page);

    // The fixture answers claude.setPermissionMode with an error by default,
    // reproducing a headless hub exactly: the brain registers no provider for
    // it (`headless_completeness_test.go:41-43`), so the promise REJECTS — and
    // a rejection never reaches the `res.ok === false` branch that would show
    // the restart confirm. It lands in a bare console.warn instead.
    const before = await page.locator('body').innerText();

    await page.getByText('Ask to approve').first().click();
    await page.getByText('Accept edits', { exact: false }).first().click();

    // The call really was attempted and really did fail…
    const call = await hub.waitForCall('claude.setPermissionMode');
    expect(call.params).toMatchObject({ sessionId: SESSION, mode: 'acceptEdits' });

    // …so something must appear on screen saying so. Deliberately loose about
    // HOW — a toast, an inline error, the restart-confirm dialog with a reason
    // all count. What must not happen is the pill quietly reverting while the
    // only trace is in devtools.
    await expect
      .poll(
        async () => {
          const now = await page.locator('body').innerText();
          const added = now
            .split('\n')
            .filter((line) => line.trim() && !before.includes(line))
            .join(' ');
          return (
            /permission|approve|accept edits|mode/i.test(added) &&
            /fail|could ?n.t|could not|unavailable|not available|refus|error|restart|unsupported/i.test(
              added,
            )
          );
        },
        { timeout: 6_000, message: 'nothing visible told the user the switch failed' },
      )
      .toBe(true);
  });
});

// ═══ gap 3 — attachments are a window.prompt for host paths ═════════════════

test.describe('composer attachments', () => {
  test('the attach control opens a file picker, not a prompt for host paths', async ({ page }) => {
    // ── Expected to fail until web-2's attachment work lands.
    test.fail(!process.env.WKS_E2E_ATTACHMENTS_FIXED);

    await openApp(page);

    const prompts: string[] = [];
    page.on('dialog', (d) => {
      prompts.push(`${d.type()}: ${d.message()}`);
      d.dismiss().catch(() => {});
    });

    const attach = page.locator('button[title*="ttach" i], button[aria-label*="ttach" i]').first();
    await expect(attach).toBeVisible({ timeout: 10_000 });

    // A real picker is the whole point: a browser cannot type a path on someone
    // else's machine, so `pickFiles`'s window.prompt is not a degraded
    // experience, it is a non-functional one.
    const chooser = page.waitForEvent('filechooser', { timeout: 5_000 }).catch(() => null);
    await attach.click();
    const got = await chooser;

    expect(prompts, 'the browser was asked to type host filesystem paths').toEqual([]);
    expect(got, 'clicking attach opened no file chooser').not.toBeNull();
  });
});
