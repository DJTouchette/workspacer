/**
 * The three things that stopped `/app` being a primary interface — now closed,
 * and guarded so they stay closed.
 *
 * `.workspacer/reports/2026-08-24-web-client-completeness.md` §0 ranked them,
 * and each is a failure a browser tab can have that an Electron shell cannot:
 *
 *   1. **Every `<webview>`-backed pane painted nothing.** Plugin panes, the
 *      Browser pane and the widget board are Electron `<webview>` elements;
 *      in Chromium that is an unknown element, so it exists, takes the right
 *      `src`, sizes itself normally — and loads nothing. The whole plugin
 *      catalogue was unreachable from a browser. Closed by web-1's iframe
 *      fallback (`72ab9a4d`).
 *   2. **A live permission-mode switch failed silently.** The rejection landed
 *      in a bare `console.warn` (`ComposerControls.tsx:474-477`), so the pill
 *      just cleared and the user learned nothing. Closed by web-2 (`a0ef7e0e`).
 *      web-4 has since given the brain a provider for the switch itself, but
 *      the rejection path stays reachable — see the spec's own comment.
 *   3. **Attaching a file was a `window.prompt` asking you to type HOST
 *      paths** (`webBackend.ts:871-883`) — not a degraded experience, a
 *      non-functional one, since a browser cannot know paths on someone else's
 *      machine. Closed by web-2 over `files.upload`.
 *
 * These specs were written against master BEFORE those fixes landed and all
 * three failed; they pass now. That order matters — a spec written after the
 * fix proves only that it can describe what it sees.
 *
 * Each asserts the USER-VISIBLE property and stays agnostic about the
 * mechanism: "content from the pane's URL is on screen" rather than
 * "an <iframe> exists", "something legible says the switch failed" rather than
 * a specific toast. A spec that pins the mechanism fails a correct rewrite.
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

// ═══ 1 — panes that load a URL must actually load it ════════════════════════

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
    await openPaneOfType(page, 'plugin', { pluginId: 'demo' });

    // Assert on CONTENT, not on the element. `<webview>` is an unknown element
    // in Chromium, so it exists, has the right `src`, sizes itself normally —
    // and loads nothing. Every check short of reading across the frame boundary
    // passes against a blank pane, which is exactly why this gap survived a
    // parity guard that checks API methods and a DOM that looks fine.
    await expect
      .poll(() => paneContentLoaded(page), {
        timeout: 8_000,
        message: 'the plugin pane loaded no content — it is a blank box',
      })
      .toBe(true);
  });

  test('the Browser pane is either usable or honest — never a silent blank', async ({ page }) => {
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

// ═══ 2 — a failure the user cannot see is a failure twice ═══════════════════

test.describe('failures the user should see', () => {
  test('a rejected permission-mode switch is surfaced, not just console.warn-ed', async ({
    page,
  }) => {
    await openApp(page);

    // The fixture answers claude.setPermissionMode with an error by default
    // (`HEADLESS_GAP_METHODS`). A REJECTION is the case that used to vanish: it
    // never reaches the `res.ok === false` path that shows the restart confirm,
    // so it fell through to a bare console.warn. web-4 has since given the brain
    // a provider, but a rejection is still reachable in a dozen ways — claudemon
    // refusing a live switch, a federated peer offline, a provider down — and
    // this spec exists to keep the client loud in every one of them.
    const before = await page.locator('body').innerText();

    await page.getByText('Ask to approve').first().click();
    await page.getByText('Accept edits', { exact: false }).first().click();

    // The call really was attempted and really did fail…
    const call = await hub.waitForCall('claude.setPermissionMode');
    expect(call.params).toMatchObject({ sessionId: SESSION, mode: 'acceptEdits' });

    // …so something must appear on screen saying so. Deliberately loose about
    // HOW — a toast, an inline error, the restart-confirm dialog with a reason
    // all count. What must not happen is the pill quietly reverting while the
    // only trace is in devtools, which is what it did until web-2's change.
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

// ═══ 3 — attachments must use the browser's own picker ═════════════════════

test.describe('composer attachments', () => {
  test('the attach control opens a file picker, not a prompt for host paths', async ({ page }) => {
    await openApp(page);

    const prompts: string[] = [];
    page.on('dialog', (d) => {
      prompts.push(`${d.type()}: ${d.message()}`);
      d.dismiss().catch(() => {});
    });

    const attach = page.locator('button[title*="ttach" i], button[aria-label*="ttach" i]').first();
    await expect(attach).toBeVisible({ timeout: 10_000 });

    // A real picker is the whole point: a browser cannot know paths on someone
    // else's machine, so the old `window.prompt` was not a degraded experience,
    // it was a non-functional one. Assert BOTH halves — no prompt, and a real
    // chooser — because dropping the prompt without adding a picker would look
    // like progress and still leave you unable to attach anything.
    const chooser = page.waitForEvent('filechooser', { timeout: 5_000 }).catch(() => null);
    await attach.click();
    const got = await chooser;

    expect(prompts, 'the browser was asked to type host filesystem paths').toEqual([]);
    expect(got, 'clicking attach opened no file chooser').not.toBeNull();
  });
});
